//! The archives the rule sets are published in, read in memory, in pure Rust: ZIP (and
//! jar) with the stored and deflate methods, gzip, and tar (ustar, pax and GNU long
//! names). Every entry name is checked when the archive is opened: an absolute path, a
//! `..` component, a backslash, a drive or a NUL refuses the whole archive. Sizes are
//! bounded:
//! an entry larger than its limit, or one that inflates to more than it declares, is
//! an error, and so is a CRC-32 that does not match.

/// The most entries an archive may have.
const MAX_ENTRIES: usize = 200_000;

/// The CRC-32 (IEEE 802.3, as ZIP and gzip use it) of `data`.
pub(crate) fn crc32(data: &[u8]) -> u32 {
    static TABLE: std::sync::OnceLock<[u32; 256]> = std::sync::OnceLock::new();
    let table = TABLE.get_or_init(|| {
        let mut table = [0u32; 256];
        for (n, slot) in (0u32..).zip(table.iter_mut()) {
            let mut c = n;
            for _ in 0..8 {
                c = if c & 1 == 1 {
                    0xEDB8_8320 ^ (c >> 1)
                } else {
                    c >> 1
                };
            }
            *slot = c;
        }
        table
    });
    !data.iter().fold(!0u32, |c, &b| {
        table[usize::from(u8::try_from((c ^ u32::from(b)) & 0xff).unwrap_or(0))] ^ (c >> 8)
    })
}

/// Checks an entry name: relative, `/`-separated, no `..`, no backslash, no drive or
/// other colon, no NUL, no empty name. `.` components and a trailing `/` are allowed.
pub(crate) fn check_name(name: &str) -> Result<(), String> {
    let bad = |why: &str| Err(format!("unsafe entry name {name:?} in the archive: {why}"));
    if name.is_empty() {
        return bad("empty");
    }
    if name.starts_with('/') {
        return bad("absolute path");
    }
    if name.contains('\\') {
        return bad("backslash");
    }
    if name.contains(':') {
        return bad("colon (a drive or a stream)");
    }
    if name.contains('\0') {
        return bad("NUL");
    }
    if name.split('/').any(|c| c == "..") {
        return bad("parent directory component");
    }
    Ok(())
}

fn u16_at(data: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(data.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(data: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(data.get(at..at + 4)?.try_into().ok()?))
}

fn usize_of(n: u32) -> usize {
    usize::try_from(n).unwrap_or(usize::MAX)
}

// --- ZIP ---------------------------------------------------------------------------

/// An entry of a ZIP archive, from its central directory.
#[derive(Debug, Clone)]
pub(crate) struct ZipEntry {
    pub(crate) name: String,
    method: u16,
    crc: u32,
    compressed: u32,
    size: u32,
    local: u32,
}

impl ZipEntry {
    /// Whether it is a directory (its name ends with `/`).
    pub(crate) fn is_dir(&self) -> bool {
        self.name.ends_with('/')
    }
}

/// A ZIP archive in memory.
#[derive(Debug)]
pub(crate) struct Zip<'a> {
    data: &'a [u8],
    entries: Vec<ZipEntry>,
}

impl<'a> Zip<'a> {
    /// Reads the central directory of `data`. ZIP64, encrypted entries and methods
    /// other than stored and deflate are refused.
    pub(crate) fn open(data: &'a [u8]) -> Result<Self, String> {
        let malformed = |what: &str| format!("not a valid ZIP archive: {what}");
        // The end of central directory record: the last signature in the last 64 KiB
        // and 22 bytes (its comment is at most 65535 bytes).
        let lowest = data.len().saturating_sub(22 + 0xFFFF);
        let eocd = (lowest..=data.len().saturating_sub(22))
            .rev()
            .find(|&i| u32_at(data, i) == Some(0x0605_4b50))
            .ok_or_else(|| malformed("no end of central directory"))?;
        let field =
            |offset: usize| u16_at(data, eocd + offset).ok_or_else(|| malformed("truncated"));
        let (disk, cd_disk, here, total) = (field(4)?, field(6)?, field(8)?, field(10)?);
        let cd_size = u32_at(data, eocd + 12).ok_or_else(|| malformed("truncated"))?;
        let cd_offset = u32_at(data, eocd + 16).ok_or_else(|| malformed("truncated"))?;
        if total == 0xFFFF || cd_size == 0xFFFF_FFFF || cd_offset == 0xFFFF_FFFF {
            return Err("ZIP64 archives are not supported".to_owned());
        }
        if disk != 0 || cd_disk != 0 || here != total {
            return Err("multi-volume ZIP archives are not supported".to_owned());
        }
        let start = usize_of(cd_offset);
        let end = start
            .checked_add(usize_of(cd_size))
            .filter(|&e| e <= eocd)
            .ok_or_else(|| malformed("central directory out of bounds"))?;
        let mut entries = Vec::with_capacity(usize::from(total));
        let mut at = start;
        for _ in 0..total {
            if entries.len() >= MAX_ENTRIES {
                return Err(format!("more than {MAX_ENTRIES} entries"));
            }
            if u32_at(data, at) != Some(0x0201_4b50) || at + 46 > end {
                return Err(malformed("bad central directory entry"));
            }
            let g16 = |o: usize| u16_at(data, at + o).unwrap_or(0);
            let g32 = |o: usize| u32_at(data, at + o).unwrap_or(0);
            let flags = g16(8);
            let method = g16(10);
            let (crc, compressed, size) = (g32(16), g32(20), g32(24));
            let (name_len, extra_len, comment_len) = (
                usize::from(g16(28)),
                usize::from(g16(30)),
                usize::from(g16(32)),
            );
            let local = g32(42);
            let name_bytes = data
                .get(at + 46..at + 46 + name_len)
                .ok_or_else(|| malformed("entry name out of bounds"))?;
            let name = String::from_utf8_lossy(name_bytes).into_owned();
            check_name(&name)?;
            if flags & 1 != 0 {
                return Err(format!("{name}: encrypted entries are not supported"));
            }
            if compressed == 0xFFFF_FFFF || size == 0xFFFF_FFFF || local == 0xFFFF_FFFF {
                return Err("ZIP64 archives are not supported".to_owned());
            }
            if method != 0 && method != 8 {
                return Err(format!(
                    "{name}: compression method {method} is not supported"
                ));
            }
            entries.push(ZipEntry {
                name,
                method,
                crc,
                compressed,
                size,
                local,
            });
            at += 46 + name_len + extra_len + comment_len;
        }
        Ok(Self { data, entries })
    }

    pub(crate) fn entries(&self) -> &[ZipEntry] {
        &self.entries
    }

    /// The entry named `name`, the first if repeated.
    pub(crate) fn entry(&self, name: &str) -> Option<&ZipEntry> {
        self.entries.iter().find(|e| e.name == name)
    }

    /// The content of `entry`, at most `limit` bytes, its CRC-32 checked.
    pub(crate) fn read(&self, entry: &ZipEntry, limit: usize) -> Result<Vec<u8>, String> {
        let name = &entry.name;
        let size = usize_of(entry.size);
        if size > limit {
            return Err(format!(
                "{name}: {size} bytes, more than the limit of {limit}"
            ));
        }
        let local = usize_of(entry.local);
        if u32_at(self.data, local) != Some(0x0403_4b50) {
            return Err(format!("{name}: bad local header"));
        }
        let name_len = usize::from(u16_at(self.data, local + 26).unwrap_or(0));
        let extra_len = usize::from(u16_at(self.data, local + 28).unwrap_or(0));
        let start = local + 30 + name_len + extra_len;
        let raw = start
            .checked_add(usize_of(entry.compressed))
            .and_then(|end| self.data.get(start..end))
            .ok_or_else(|| format!("{name}: data out of bounds"))?;
        let content = if entry.method == 0 {
            raw.to_vec()
        } else {
            miniz_oxide::inflate::decompress_to_vec_with_limit(raw, size)
                .map_err(|e| format!("{name}: cannot inflate: {:?}", e.status))?
        };
        if content.len() != size {
            return Err(format!(
                "{name}: {} bytes, the archive declares {size}",
                content.len()
            ));
        }
        if crc32(&content) != entry.crc {
            return Err(format!("{name}: CRC-32 mismatch"));
        }
        Ok(content)
    }
}

// --- gzip --------------------------------------------------------------------------

/// The content of a gzip file (one member), at most `limit` bytes, its CRC-32 and
/// length checked.
pub(crate) fn gunzip(data: &[u8], limit: usize) -> Result<Vec<u8>, String> {
    let malformed = |what: &str| format!("not a valid gzip file: {what}");
    if data.len() < 18 || data[0] != 0x1f || data[1] != 0x8b {
        return Err(malformed("no gzip header"));
    }
    if data[2] != 8 {
        return Err(malformed("not deflate"));
    }
    let flags = data[3];
    if flags & 0xE0 != 0 {
        return Err(malformed("reserved flags set"));
    }
    let mut at = 10;
    if flags & 4 != 0 {
        let extra = usize::from(u16_at(data, at).ok_or_else(|| malformed("truncated"))?);
        at += 2 + extra;
    }
    for flag in [8u8, 16] {
        if flags & flag != 0 {
            let end = data
                .get(at..)
                .and_then(|rest| rest.iter().position(|&b| b == 0))
                .ok_or_else(|| malformed("unterminated name or comment"))?;
            at += end + 1;
        }
    }
    if flags & 2 != 0 {
        at += 2;
    }
    let body = data
        .get(at..data.len() - 8)
        .ok_or_else(|| malformed("truncated"))?;
    let content = miniz_oxide::inflate::decompress_to_vec_with_limit(body, limit).map_err(|e| {
        if e.status == miniz_oxide::inflate::TINFLStatus::HasMoreOutput {
            format!("more than the limit of {limit} bytes once decompressed")
        } else {
            malformed(&format!("cannot inflate: {:?}", e.status))
        }
    })?;
    let crc = u32_at(data, data.len() - 8).unwrap_or(0);
    let length = u32_at(data, data.len() - 4).unwrap_or(0);
    if crc32(&content) != crc || (content.len() as u64 & 0xFFFF_FFFF) != u64::from(length) {
        return Err(malformed(
            "CRC-32 or length mismatch (truncated, or several members)",
        ));
    }
    Ok(content)
}

// --- tar ---------------------------------------------------------------------------

/// The kind of a tar entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TarKind {
    File,
    Directory,
    /// A hard or symbolic link: never extracted.
    Link,
    /// Anything else (devices, FIFOs): never extracted.
    Other,
}

/// An entry of a tar archive.
#[derive(Debug, Clone)]
pub(crate) struct TarEntry {
    pub(crate) name: String,
    pub(crate) kind: TarKind,
    start: usize,
    size: usize,
}

/// A tar archive in memory.
#[derive(Debug)]
pub(crate) struct Tar<'a> {
    data: &'a [u8],
    entries: Vec<TarEntry>,
    /// The `comment` of the pax global header that starts the archive, if any: the
    /// commit id `git archive` records (what `git get-tar-commit-id` prints).
    pub(crate) commit_comment: Option<String>,
}

fn octal(field: &[u8]) -> Result<u64, String> {
    // GNU base-256 for large values.
    if field.first().is_some_and(|b| b & 0x80 != 0) {
        let mut value: u64 = u64::from(field[0] & 0x7f);
        for &b in &field[1..] {
            value = value
                .checked_mul(256)
                .and_then(|v| v.checked_add(u64::from(b)))
                .ok_or("tar number too large")?;
        }
        return Ok(value);
    }
    let text = field
        .iter()
        .take_while(|&&b| b != 0)
        .map(|&b| char::from(b))
        .collect::<String>();
    let text = text.trim();
    if text.is_empty() {
        return Ok(0);
    }
    u64::from_str_radix(text, 8).map_err(|_| format!("bad tar number {text:?}"))
}

fn text(field: &[u8]) -> String {
    let end = field.iter().position(|&b| b == 0).unwrap_or(field.len());
    String::from_utf8_lossy(&field[..end]).into_owned()
}

/// The records of a pax header: `length key=value\n`.
fn pax_records(data: &[u8]) -> Result<Vec<(String, String)>, String> {
    let mut records = Vec::new();
    let mut rest = data;
    while !rest.is_empty() {
        if rest.iter().all(|&b| b == 0) {
            break;
        }
        let space = rest
            .iter()
            .position(|&b| b == b' ')
            .ok_or("bad pax record")?;
        let length: usize = std::str::from_utf8(&rest[..space])
            .ok()
            .and_then(|s| s.parse().ok())
            .ok_or("bad pax record length")?;
        let record = rest.get(space + 1..length).ok_or("truncated pax record")?;
        let record = record
            .strip_suffix(b"\n")
            .ok_or("pax record without a line end")?;
        let eq = record
            .iter()
            .position(|&b| b == b'=')
            .ok_or("pax record without =")?;
        records.push((
            String::from_utf8_lossy(&record[..eq]).into_owned(),
            String::from_utf8_lossy(&record[eq + 1..]).into_owned(),
        ));
        rest = &rest[length..];
    }
    Ok(records)
}

/// Checks the checksum of the tar header at byte `at`: the sum of its bytes, its
/// checksum field counted as spaces.
fn check_header(header: &[u8], at: usize) -> Result<(), String> {
    let stored = octal(&header[148..156])?;
    let sum: u64 = header
        .iter()
        .enumerate()
        .map(|(i, &b)| {
            if (148..156).contains(&i) {
                32
            } else {
                u64::from(b)
            }
        })
        .sum();
    if stored == sum {
        Ok(())
    } else {
        Err(format!(
            "not a valid tar archive: bad header checksum at byte {at}"
        ))
    }
}

impl<'a> Tar<'a> {
    /// Reads the headers of the tar archive `data`, checking each header's checksum and
    /// each entry name.
    pub(crate) fn open(data: &'a [u8]) -> Result<Self, String> {
        let mut entries = Vec::new();
        let mut commit_comment = None;
        let mut at = 0;
        let mut long_name: Option<String> = None;
        let mut pax_path: Option<String> = None;
        let mut pax_size: Option<u64> = None;
        let mut first = true;
        while at + 512 <= data.len() {
            let header = &data[at..at + 512];
            if header.iter().all(|&b| b == 0) {
                break;
            }
            check_header(header, at)?;
            let size = pax_size
                .take()
                .map_or_else(|| octal(&header[124..136]), Ok)?;
            let size = usize::try_from(size).map_err(|_| "tar entry too large".to_owned())?;
            let start = at + 512;
            let end = start
                .checked_add(size)
                .filter(|&e| e <= data.len())
                .ok_or_else(|| "not a valid tar archive: entry out of bounds".to_owned())?;
            let typeflag = header[156];
            let ustar = &header[257..262] == b"ustar";
            match typeflag {
                b'g' => {
                    let records = pax_records(&data[start..end])?;
                    if first {
                        commit_comment = records
                            .into_iter()
                            .find(|(k, _)| k == "comment")
                            .map(|(_, v)| v);
                    }
                }
                b'x' => {
                    for (key, value) in pax_records(&data[start..end])? {
                        match key.as_str() {
                            "path" => pax_path = Some(value),
                            "size" => {
                                pax_size = Some(value.parse().map_err(|_| "bad pax size")?);
                            }
                            _ => {}
                        }
                    }
                }
                b'L' => long_name = Some(text(&data[start..end])),
                b'K' => {}
                _ => {
                    let mut name = text(&header[0..100]);
                    if ustar {
                        let prefix = text(&header[345..500]);
                        if !prefix.is_empty() {
                            name = format!("{prefix}/{name}");
                        }
                    }
                    if let Some(n) = long_name.take() {
                        name = n;
                    }
                    if let Some(n) = pax_path.take() {
                        name = n;
                    }
                    check_name(&name)?;
                    let kind = match typeflag {
                        b'0' | 0 | b'7' => TarKind::File,
                        b'5' => TarKind::Directory,
                        b'1' | b'2' => TarKind::Link,
                        _ => TarKind::Other,
                    };
                    if entries.len() >= MAX_ENTRIES {
                        return Err(format!("more than {MAX_ENTRIES} entries"));
                    }
                    entries.push(TarEntry {
                        name,
                        kind,
                        start,
                        size,
                    });
                }
            }
            first = false;
            at = start + size.div_ceil(512) * 512;
        }
        Ok(Self {
            data,
            entries,
            commit_comment,
        })
    }

    #[cfg(test)]
    pub(crate) fn entries(&self) -> &[TarEntry] {
        &self.entries
    }

    /// The content of a file entry.
    pub(crate) fn content(&self, entry: &TarEntry) -> &'a [u8] {
        self.data
            .get(entry.start..entry.start + entry.size)
            .unwrap_or(&[])
    }

    /// The files whose name, without its first component (`tar --strip-components=1`),
    /// starts with `prefix`: that name and the content of each, in archive order.
    pub(crate) fn stripped_under(&self, prefix: &str) -> Vec<(&str, &'a [u8])> {
        self.entries
            .iter()
            .filter(|e| e.kind == TarKind::File)
            .filter_map(|e| {
                let (_, rest) = e.name.split_once('/')?;
                rest.starts_with(prefix).then(|| (rest, self.content(e)))
            })
            .collect()
    }

    /// The file whose name, without its first component (`tar --strip-components=1`),
    /// is `name`.
    pub(crate) fn stripped(&self, name: &str) -> Option<&'a [u8]> {
        self.entries
            .iter()
            .find(|e| {
                e.kind == TarKind::File
                    && e.name.split_once('/').is_some_and(|(_, rest)| rest == name)
            })
            .map(|e| self.content(e))
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A ZIP archive of `files` (name, content, deflated?), as a test fixture.
    pub(crate) fn zip(files: &[(&str, &[u8], bool)]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut central = Vec::new();
        for (name, content, deflate) in files {
            let data = if *deflate {
                miniz_oxide::deflate::compress_to_vec(content, 6)
            } else {
                content.to_vec()
            };
            let crc = crc32(content);
            let offset = u32::try_from(out.len()).unwrap();
            let method: u16 = if *deflate { 8 } else { 0 };
            let n = u16::try_from(name.len()).unwrap();
            let (c, u) = (
                u32::try_from(data.len()).unwrap(),
                u32::try_from(content.len()).unwrap(),
            );
            out.extend(0x0403_4b50u32.to_le_bytes());
            out.extend(20u16.to_le_bytes());
            out.extend(0u16.to_le_bytes());
            out.extend(method.to_le_bytes());
            out.extend([0u8; 4]);
            out.extend(crc.to_le_bytes());
            out.extend(c.to_le_bytes());
            out.extend(u.to_le_bytes());
            out.extend(n.to_le_bytes());
            out.extend(0u16.to_le_bytes());
            out.extend(name.as_bytes());
            out.extend(&data);
            central.extend(0x0201_4b50u32.to_le_bytes());
            central.extend(20u16.to_le_bytes());
            central.extend(20u16.to_le_bytes());
            central.extend(0u16.to_le_bytes());
            central.extend(method.to_le_bytes());
            central.extend([0u8; 4]);
            central.extend(crc.to_le_bytes());
            central.extend(c.to_le_bytes());
            central.extend(u.to_le_bytes());
            central.extend(n.to_le_bytes());
            central.extend([0u8; 12]);
            central.extend(offset.to_le_bytes());
            central.extend(name.as_bytes());
        }
        let cd_offset = u32::try_from(out.len()).unwrap();
        let count = u16::try_from(files.len()).unwrap();
        out.extend(&central);
        out.extend(0x0605_4b50u32.to_le_bytes());
        out.extend([0u8; 4]);
        out.extend(count.to_le_bytes());
        out.extend(count.to_le_bytes());
        out.extend(u32::try_from(central.len()).unwrap().to_le_bytes());
        out.extend(cd_offset.to_le_bytes());
        out.extend(0u16.to_le_bytes());
        out
    }

    /// A tar header block for `name`.
    fn tar_header(name: &str, size: usize, typeflag: u8) -> Vec<u8> {
        let mut h = vec![0u8; 512];
        h[..name.len()].copy_from_slice(name.as_bytes());
        h[100..107].copy_from_slice(b"0000644");
        h[124..135].copy_from_slice(format!("{size:011o}").as_bytes());
        h[156] = typeflag;
        h[257..263].copy_from_slice(b"ustar\0");
        h[263..265].copy_from_slice(b"00");
        h[148..156].copy_from_slice(b"        ");
        let sum: u32 = h.iter().map(|&b| u32::from(b)).sum();
        h[148..155].copy_from_slice(format!("{sum:06o}\0").as_bytes());
        h
    }

    fn pad(out: &mut Vec<u8>) {
        while !out.len().is_multiple_of(512) {
            out.push(0);
        }
    }

    /// A tar archive of `entries` (name, content, typeflag), with a pax global header
    /// recording `comment` first when given, as a test fixture.
    pub(crate) fn tar(comment: Option<&str>, entries: &[(&str, &[u8], u8)]) -> Vec<u8> {
        let mut out = Vec::new();
        if let Some(comment) = comment {
            let body = format!("comment={comment}\n");
            // The length counts itself.
            let mut length = body.len() + 2;
            if format!("{length} {body}").len() != length {
                length += 1;
            }
            let record = format!("{length} {body}");
            out.extend(tar_header("pax_global_header", record.len(), b'g'));
            out.extend(record.as_bytes());
            pad(&mut out);
        }
        for (name, content, typeflag) in entries {
            out.extend(tar_header(name, content.len(), *typeflag));
            out.extend(*content);
            pad(&mut out);
        }
        out.extend([0u8; 1024]);
        out
    }

    /// A gzip file of `data`.
    pub(crate) fn gzip(data: &[u8]) -> Vec<u8> {
        let mut out = vec![0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 3];
        out.extend(miniz_oxide::deflate::compress_to_vec(data, 6));
        out.extend(crc32(data).to_le_bytes());
        out.extend(u32::try_from(data.len()).unwrap().to_le_bytes());
        out
    }

    #[test]
    fn crc32_of_known_values() {
        assert_eq!(crc32(b""), 0);
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(
            crc32(b"The quick brown fox jumps over the lazy dog"),
            0x414F_A339
        );
    }

    #[test]
    fn reads_zip_entries() {
        let big = b"abc".repeat(1000);
        let data = zip(&[
            ("a/", b"", false),
            ("a/one.txt", b"one", false),
            ("a/big.xml", &big, true),
        ]);
        let archive = Zip::open(&data).unwrap();
        assert_eq!(archive.entries().len(), 3);
        assert!(archive.entry("a/").unwrap().is_dir());
        assert_eq!(
            archive
                .read(archive.entry("a/one.txt").unwrap(), 10)
                .unwrap(),
            b"one"
        );
        let entry = archive.entry("a/big.xml").unwrap();
        assert_eq!(archive.read(entry, 3000).unwrap(), big);
        assert!(
            archive
                .read(entry, 2999)
                .unwrap_err()
                .contains("more than the limit")
        );
        assert!(archive.entry("nope").is_none());
    }

    #[test]
    fn refuses_corrupt_zip_entries() {
        let mut data = zip(&[("x.txt", b"hello world", false)]);
        // Flip a byte of the content: the CRC-32 no longer matches.
        let at = data.windows(5).position(|w| w == b"hello").unwrap();
        data[at] = b'j';
        let archive = Zip::open(&data).unwrap();
        assert!(
            archive
                .read(&archive.entries()[0], 100)
                .unwrap_err()
                .contains("CRC-32")
        );
        assert!(
            Zip::open(b"not a zip")
                .unwrap_err()
                .contains("not a valid ZIP")
        );
        assert!(Zip::open(&data[..data.len() - 30]).is_err());
    }

    #[test]
    fn refuses_zip_bombs() {
        // An entry that inflates to more than it declares.
        let content = vec![b'a'; 100_000];
        let mut data = zip(&[("bomb.xml", &content, true)]);
        // Declare 1000 bytes in the central directory.
        let central = data
            .windows(4)
            .position(|w| w == 0x0201_4b50u32.to_le_bytes())
            .unwrap();
        data[central + 24..central + 28].copy_from_slice(&1000u32.to_le_bytes());
        let archive = Zip::open(&data).unwrap();
        let error = archive.read(&archive.entries()[0], 1 << 20).unwrap_err();
        assert!(
            error.contains("cannot inflate") || error.contains("declares"),
            "{error}"
        );
    }

    #[test]
    fn refuses_traversal_in_zip_names() {
        for name in [
            "../evil.xml",
            "a/../../evil.xml",
            "/etc/passwd",
            "a\\..\\evil.xml",
            "C:/evil.xml",
            "a/\0b",
            "",
        ] {
            let data = zip(&[("ok.txt", b"ok", false), (name, b"evil", false)]);
            let error = Zip::open(&data).unwrap_err();
            assert!(error.contains("unsafe entry name"), "{name:?}: {error}");
        }
        // Dots that are not a parent component are fine.
        assert!(Zip::open(&zip(&[("a/..b/c..", b"", false), ("./d", b"", false)])).is_ok());
    }

    #[test]
    fn reads_gzip_and_refuses_corruption() {
        let data = b"some text ".repeat(100);
        let gz = gzip(&data);
        assert_eq!(gunzip(&gz, 1000).unwrap(), data);
        assert!(gunzip(&gz, 999).unwrap_err().contains("limit"));
        let mut bad = gz.clone();
        let last = bad.len() - 5;
        bad[last] ^= 1;
        assert!(gunzip(&bad, 1 << 20).unwrap_err().contains("mismatch"));
        assert!(gunzip(&gz[..gz.len() / 2], 1 << 20).is_err());
        assert!(gunzip(b"plain", 10).is_err());
    }

    #[test]
    fn reads_tar_with_a_commit_comment() {
        let data = tar(
            Some("38d693d71d28d813781323a6ca97324edcc4d239"),
            &[
                ("repo-1/", b"", b'5'),
                ("repo-1/rules/sch/A.sch", b"<schema/>", b'0'),
                ("repo-1/README", b"readme", b'0'),
            ],
        );
        let archive = Tar::open(&data).unwrap();
        assert_eq!(
            archive.commit_comment.as_deref(),
            Some("38d693d71d28d813781323a6ca97324edcc4d239")
        );
        assert_eq!(archive.entries().len(), 3);
        assert_eq!(archive.stripped("rules/sch/A.sch"), Some(&b"<schema/>"[..]));
        assert_eq!(archive.stripped("README"), Some(&b"readme"[..]));
        assert_eq!(archive.stripped("repo-1/README"), None);
        // Without a global header first, no commit.
        let plain = tar(None, &[("a/b", b"x", b'0')]);
        assert_eq!(Tar::open(&plain).unwrap().commit_comment, None);
    }

    #[test]
    fn refuses_a_bad_tar_checksum() {
        let mut data = tar(None, &[("a/b", b"x", b'0')]);
        data[0] = b'z';
        assert!(Tar::open(&data).unwrap_err().contains("checksum"));
    }

    #[test]
    fn refuses_traversal_in_tar_names() {
        for name in ["../evil", "a/../../evil", "/abs/evil", "a\\b"] {
            let data = tar(None, &[(name, b"evil", b'0')]);
            assert!(
                Tar::open(&data).unwrap_err().contains("unsafe entry name"),
                "{name}"
            );
        }
        // Links and directories are listed, never read as files.
        let data = tar(
            None,
            &[
                ("m/", b"", b'5'),
                ("m/link", b"", b'2'),
                ("m/lib/a.jar", b"jar", b'0'),
            ],
        );
        let archive = Tar::open(&data).unwrap();
        assert_eq!(archive.stripped("link"), None);
        assert_eq!(archive.stripped("lib/a.jar"), Some(&b"jar"[..]));
        assert_eq!(archive.stripped_under("li"), [("lib/a.jar", &b"jar"[..])]);
    }

    #[test]
    fn reads_pax_and_gnu_long_names() {
        let long = format!("top/{}/file.txt", "d".repeat(150));
        let record_body = format!("path={long}\n");
        let mut length = record_body.len() + 4;
        if format!("{length} {record_body}").len() != length {
            length += 1;
        }
        let record = format!("{length} {record_body}");
        let mut data = Vec::new();
        data.extend(tar_header("PaxHeader", record.len(), b'x'));
        data.extend(record.as_bytes());
        pad(&mut data);
        data.extend(tar_header("short-name", 3, b'0'));
        data.extend(b"pax");
        pad(&mut data);
        let gnu = format!("top/{}/gnu.txt", "g".repeat(150));
        data.extend(tar_header("././@LongLink", gnu.len() + 1, b'L'));
        data.extend(gnu.as_bytes());
        data.push(0);
        pad(&mut data);
        data.extend(tar_header("short", 3, b'0'));
        data.extend(b"gnu");
        pad(&mut data);
        data.extend([0u8; 1024]);
        let archive = Tar::open(&data).unwrap();
        let names: Vec<&str> = archive.entries().iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, [long.as_str(), gnu.as_str()]);
        assert_eq!(archive.content(&archive.entries()[0]), b"pax");
        assert_eq!(archive.content(&archive.entries()[1]), b"gnu");
    }
}
