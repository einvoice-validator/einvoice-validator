//! Stream filters (ISO 32000-2, 7.4): `FlateDecode` with the PNG predictors (7.4.4.4).
//! Every other filter, and the TIFF predictor, is refused.

use miniz_oxide::inflate::{TINFLStatus, decompress_to_vec_zlib_with_limit};

use crate::error::{Error, Result};
use crate::object::{Dict, Object, display_name};

/// One filter of a stream's `/Filter`, with its `/DecodeParms` (references resolved).
#[derive(Debug, Clone)]
pub(crate) struct Filter {
    pub(crate) name: Vec<u8>,
    pub(crate) params: Option<Dict>,
}

/// The decoding budget of one document: the size of any decoded stream, and the total
/// decoded so far, are bounded by [`Limits`](crate::Limits).
#[derive(Debug)]
pub(crate) struct Budget {
    used: usize,
    max_total: usize,
    max_stream: usize,
}

impl Budget {
    pub(crate) fn new(limits: &crate::Limits) -> Self {
        Self {
            used: 0,
            max_total: limits.max_total_decoded,
            max_stream: limits.max_stream_size,
        }
    }

    /// [`decode`], counted against the budget.
    pub(crate) fn decode(&mut self, data: &[u8], filters: &[Filter]) -> Result<Vec<u8>> {
        let remaining = self.max_total.saturating_sub(self.used);
        let total_bound = remaining < self.max_stream;
        let limit = remaining.min(self.max_stream);
        let out = decode(data, filters, limit).map_err(|error| match error {
            Error::LimitExceeded { .. } if total_bound => Error::LimitExceeded {
                limit: "max_total_decoded",
                value: self.max_total,
            },
            Error::LimitExceeded { .. } => Error::LimitExceeded {
                limit: "max_stream_size",
                value: self.max_stream,
            },
            other => other,
        })?;
        self.used += out.len();
        Ok(out)
    }
}

/// The filters of a stream dictionary (`/Filter` and `/DecodeParms`), each value passed
/// through `resolve` (which resolves indirect references, or refuses them).
pub(crate) fn filters(
    dict: &Dict,
    resolve: &mut dyn FnMut(&Object) -> Result<Object>,
) -> Result<Vec<Filter>> {
    for key in [b"F".as_slice(), b"FFilter", b"FDecodeParms"] {
        if dict.get(key).is_some() {
            return Err(Error::unsupported(
                None,
                format!("stream data in an external file ({})", display_name(key)),
            ));
        }
    }
    let names = match dict.get(b"Filter").map(&mut *resolve).transpose()? {
        None => Vec::new(),
        Some(Object::Name(name)) => vec![name],
        Some(Object::Array(items)) => items
            .iter()
            .map(|item| match resolve(item)? {
                Object::Name(name) => Ok(name),
                other => Err(Error::malformed(
                    None,
                    format!("an element of /Filter is {}, not a name", other.kind()),
                )),
            })
            .collect::<Result<_>>()?,
        Some(other) => {
            return Err(Error::malformed(
                None,
                format!("/Filter is {}, not a name or an array", other.kind()),
            ));
        }
    };
    let as_params = |object: Object| match object {
        Object::Null => Ok(None),
        Object::Dict(params) => Ok(Some(params)),
        other => Err(Error::malformed(
            None,
            format!("/DecodeParms is {}, not a dictionary", other.kind()),
        )),
    };
    let params: Vec<Option<Dict>> = match dict.get(b"DecodeParms").map(&mut *resolve).transpose()? {
        None => vec![None; names.len()],
        Some(Object::Array(items)) => {
            let params = items
                .iter()
                .map(|item| as_params(resolve(item)?))
                .collect::<Result<Vec<_>>>()?;
            if params.len() != names.len() {
                return Err(Error::malformed(
                    None,
                    "/DecodeParms and /Filter have different lengths",
                ));
            }
            params
        }
        Some(single) => {
            if names.len() != 1 {
                return Err(Error::malformed(
                    None,
                    "a single /DecodeParms dictionary for several filters",
                ));
            }
            vec![as_params(single)?]
        }
    };
    Ok(names
        .into_iter()
        .zip(params)
        .map(|(name, params)| Filter { name, params })
        .collect())
}

/// Applies `filters` in order to `data`; no output may exceed `limit` bytes.
pub(crate) fn decode(data: &[u8], filters: &[Filter], limit: usize) -> Result<Vec<u8>> {
    let mut current: Option<Vec<u8>> = None;
    for filter in filters {
        let input = current.as_deref().unwrap_or(data);
        let output = match filter.name.as_slice() {
            b"FlateDecode" => {
                let inflated = inflate(input, limit)?;
                match &filter.params {
                    Some(params) => predict(inflated, params)?,
                    None => inflated,
                }
            }
            other => {
                return Err(Error::unsupported(
                    None,
                    format!("stream filter {}", display_name(other)),
                ));
            }
        };
        current = Some(output);
    }
    let output = current.unwrap_or_else(|| data.to_vec());
    if output.len() > limit {
        return Err(Error::LimitExceeded {
            limit: "max_stream_size",
            value: limit,
        });
    }
    Ok(output)
}

/// zlib-wrapped DEFLATE data (RFC 1950, RFC 1951), Adler-32 checksum verified.
fn inflate(data: &[u8], limit: usize) -> Result<Vec<u8>> {
    decompress_to_vec_zlib_with_limit(data, limit).map_err(|error| match error.status {
        TINFLStatus::HasMoreOutput => Error::LimitExceeded {
            limit: "max_stream_size",
            value: limit,
        },
        TINFLStatus::Adler32Mismatch => {
            Error::malformed(None, "FlateDecode data: Adler-32 checksum mismatch")
        }
        TINFLStatus::FailedCannotMakeProgress | TINFLStatus::NeedsMoreInput => {
            Error::malformed(None, "FlateDecode data: truncated")
        }
        status => Error::malformed(None, format!("FlateDecode data: {status:?}")),
    })
}

fn integer_param(params: &Dict, key: &[u8], default: i64) -> Result<i64> {
    match params.get(key) {
        None => Ok(default),
        Some(Object::Integer(value)) => Ok(*value),
        Some(other) => Err(Error::malformed(
            None,
            format!(
                "/DecodeParms {} is {}, not an integer",
                display_name(key),
                other.kind()
            ),
        )),
    }
}

/// Undoes the predictor of `/DecodeParms`, if any.
fn predict(data: Vec<u8>, params: &Dict) -> Result<Vec<u8>> {
    let predictor = integer_param(params, b"Predictor", 1)?;
    match predictor {
        1 => Ok(data),
        2 => Err(Error::unsupported(None, "TIFF predictor (/Predictor 2)")),
        10..=15 => png(&data, params),
        other => Err(Error::malformed(
            None,
            format!("/Predictor {other} is not 1, 2 or 10 to 15"),
        )),
    }
}

/// The PNG predictors: each row starts with its filter type byte (PNG specification,
/// clause 9), whatever /Predictor 10 to 15 says.
fn png(data: &[u8], params: &Dict) -> Result<Vec<u8>> {
    let colors = integer_param(params, b"Colors", 1)?;
    let bits = integer_param(params, b"BitsPerComponent", 8)?;
    let columns = integer_param(params, b"Columns", 1)?;
    if !(1..=32).contains(&colors) {
        return Err(Error::malformed(
            None,
            format!("/Colors {colors} is not between 1 and 32"),
        ));
    }
    if ![1, 2, 4, 8, 16].contains(&bits) {
        return Err(Error::malformed(
            None,
            format!("/BitsPerComponent {bits} is not 1, 2, 4, 8 or 16"),
        ));
    }
    let too_wide = || Error::malformed(None, format!("/Columns {columns} out of range"));
    let columns = usize::try_from(columns).map_err(|_| too_wide())?;
    if columns == 0 {
        return Err(too_wide());
    }
    // Both factors are at most 32 and 16, so these conversions and products are exact.
    let pixel_bits = usize::try_from(colors * bits).map_err(|_| too_wide())?;
    let row_bits = pixel_bits.checked_mul(columns).ok_or_else(too_wide)?;
    let row = row_bits.div_ceil(8);
    let bpp = pixel_bits.div_ceil(8);
    let stride = row.checked_add(1).ok_or_else(too_wide)?;
    if !data.len().is_multiple_of(stride) {
        return Err(Error::malformed(
            None,
            format!(
                "PNG predictor: {} bytes is not a whole number of rows of {stride} bytes",
                data.len()
            ),
        ));
    }
    let mut out = Vec::with_capacity(data.len() / stride * row);
    let mut previous = vec![0u8; row];
    let mut current = vec![0u8; row];
    for chunk in data.chunks_exact(stride) {
        let (kind, bytes) = (chunk[0], &chunk[1..]);
        for i in 0..row {
            let left = if i >= bpp { current[i - bpp] } else { 0 };
            let up = previous[i];
            let up_left = if i >= bpp { previous[i - bpp] } else { 0 };
            let prediction = match kind {
                0 => 0,
                1 => left,
                2 => up,
                3 => u8::midpoint(left, up),
                4 => paeth(left, up, up_left),
                other => {
                    return Err(Error::malformed(
                        None,
                        format!("PNG predictor: row filter type {other} is not 0 to 4"),
                    ));
                }
            };
            current[i] = bytes[i].wrapping_add(prediction);
        }
        out.extend_from_slice(&current);
        std::mem::swap(&mut previous, &mut current);
    }
    Ok(out)
}

fn paeth(a: u8, b: u8, c: u8) -> u8 {
    let p = i16::from(a) + i16::from(b) - i16::from(c);
    let pa = (p - i16::from(a)).abs();
    let pb = (p - i16::from(b)).abs();
    let pc = (p - i16::from(c)).abs();
    if pa <= pb && pa <= pc {
        a
    } else if pb <= pc {
        b
    } else {
        c
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::Parser;
    use miniz_oxide::deflate::compress_to_vec_zlib;

    fn params(text: &str) -> Dict {
        match Parser::new(text.as_bytes(), 0, 8).object().unwrap() {
            Object::Dict(d) => d,
            other => panic!("{other:?}"),
        }
    }

    fn flate(params: Option<Dict>) -> Vec<Filter> {
        vec![Filter {
            name: b"FlateDecode".to_vec(),
            params,
        }]
    }

    #[test]
    fn inflates() {
        let data = b"hello hello hello hello".repeat(10);
        let compressed = compress_to_vec_zlib(&data, 6);
        assert_eq!(decode(&compressed, &flate(None), 1 << 20).unwrap(), data);
        assert_eq!(decode(&data, &[], 1 << 20).unwrap(), data);
        // Two Flate filters in a row.
        let twice = compress_to_vec_zlib(&compressed, 6);
        let filters = [flate(None), flate(None)].concat();
        assert_eq!(decode(&twice, &filters, 1 << 20).unwrap(), data);
    }

    #[test]
    fn refuses_bad_flate() {
        let data = b"some text to compress".repeat(5);
        let compressed = compress_to_vec_zlib(&data, 6);
        assert!(decode(&compressed[..compressed.len() - 6], &flate(None), 1 << 20).is_err());
        let mut bad_sum = compressed.clone();
        let last = bad_sum.len() - 1;
        bad_sum[last] ^= 1;
        assert!(decode(&bad_sum, &flate(None), 1 << 20).is_err());
        assert!(decode(b"not zlib", &flate(None), 1 << 20).is_err());
        assert!(matches!(
            decode(&compressed, &flate(None), 10),
            Err(Error::LimitExceeded { .. })
        ));
        assert!(matches!(
            decode(&data, &[], 10),
            Err(Error::LimitExceeded { .. })
        ));
        let other = [Filter {
            name: b"LZWDecode".to_vec(),
            params: None,
        }];
        assert!(matches!(
            decode(&data, &other, 1 << 20),
            Err(Error::Unsupported { .. })
        ));
    }

    /// Encodes `rows` with the PNG filter `kind` on every row, `bpp` bytes per pixel.
    fn png_encode(rows: &[Vec<u8>], kind: u8, bpp: usize) -> Vec<u8> {
        let mut out = Vec::new();
        let width = rows[0].len();
        let zero = vec![0u8; width];
        for (r, row) in rows.iter().enumerate() {
            let prev = if r == 0 { &zero } else { &rows[r - 1] };
            out.push(kind);
            for i in 0..width {
                let a = if i >= bpp { row[i - bpp] } else { 0 };
                let b = prev[i];
                let c = if i >= bpp { prev[i - bpp] } else { 0 };
                let p = match kind {
                    0 => 0,
                    1 => a,
                    2 => b,
                    3 => u8::midpoint(a, b),
                    _ => paeth(a, b, c),
                };
                out.push(row[i].wrapping_sub(p));
            }
        }
        out
    }

    #[test]
    fn png_predictors() {
        let rows: Vec<Vec<u8>> = (0u8..6)
            .map(|r| {
                (0u8..8)
                    .map(|c| r.wrapping_mul(37).wrapping_add(c.wrapping_mul(91)))
                    .collect()
            })
            .collect();
        let flat: Vec<u8> = rows.concat();
        for kind in 0..=4 {
            for (colors, bpp) in [(1, 1), (2, 2), (4, 4)] {
                let encoded = png_encode(&rows, kind, bpp);
                let columns = 8 / bpp;
                let p = params(&format!(
                    "<< /Predictor 12 /Colors {colors} /Columns {columns} >>"
                ));
                let compressed = compress_to_vec_zlib(&encoded, 6);
                assert_eq!(
                    decode(&compressed, &flate(Some(p)), 1 << 20).unwrap(),
                    flat,
                    "filter {kind}, {colors} colors"
                );
            }
        }
    }

    #[test]
    fn png_refusals() {
        let rows = vec![vec![1u8, 2, 3, 4]];
        let encoded = png_encode(&rows, 2, 1);
        let compressed = compress_to_vec_zlib(&encoded, 6);
        for bad in [
            "<< /Predictor 12 /Columns 3 >>",
            "<< /Predictor 12 /Columns 0 >>",
            "<< /Predictor 12 /Columns -4 >>",
            "<< /Predictor 12 /Columns 4 /Colors 0 >>",
            "<< /Predictor 12 /Columns 4 /BitsPerComponent 3 >>",
            "<< /Predictor 7 /Columns 4 >>",
            "<< /Predictor 2 /Columns 4 >>",
            "<< /Predictor /Up >>",
        ] {
            assert!(
                decode(&compressed, &flate(Some(params(bad))), 1 << 20).is_err(),
                "{bad}"
            );
        }
        let ok = params("<< /Predictor 12 /Columns 4 >>");
        assert_eq!(
            decode(&compressed, &flate(Some(ok.clone())), 1 << 20).unwrap(),
            vec![1, 2, 3, 4]
        );
        let mut bad_kind = encoded.clone();
        bad_kind[0] = 5;
        let compressed = compress_to_vec_zlib(&bad_kind, 6);
        assert!(decode(&compressed, &flate(Some(ok)), 1 << 20).is_err());
    }
}
