//! A writer of small synthetic PDF files, and Factur-X-like fixtures built with it: classic
//! cross-reference tables, cross-reference streams (with or without the PNG Up
//! predictor), object streams, hybrid files and incremental updates.

#![allow(
    dead_code,
    reason = "shared by several test targets, each using a part"
)]

use std::collections::BTreeMap;
use std::fmt::Write as _;

use miniz_oxide::deflate::compress_to_vec_zlib;

/// The synthetic invoice of `tests/data/invoice.xml`.
pub(crate) const INVOICE: &[u8] = include_bytes!("../data/invoice.xml");
/// Its MD5 digest (`md5sum tests/data/invoice.xml`).
pub(crate) const INVOICE_MD5: &str = "e8c96c8e43f89c84c7e316693f242973";

pub(crate) fn deflate(data: &[u8]) -> Vec<u8> {
    compress_to_vec_zlib(data, 6)
}

/// A PDF being written, object by object, then closed by cross-reference sections.
#[derive(Debug)]
pub(crate) struct Writer {
    pub(crate) out: Vec<u8>,
    /// Objects written since the last section: offset and generation.
    pending: BTreeMap<u32, (usize, u16)>,
    /// Objects put in object streams since the last section: (stream, index).
    compressed: BTreeMap<u32, (u32, u32)>,
    /// The offset of the last section, for `/Prev`.
    prev: Option<usize>,
    /// One more than the highest object number written.
    size: u32,
}

impl Writer {
    pub(crate) fn new(version: &str) -> Self {
        let mut out = format!("%PDF-{version}\n%").into_bytes();
        out.extend_from_slice(&[0xE2, 0xE3, 0xCF, 0xD3, b'\n']);
        Self {
            out,
            pending: BTreeMap::new(),
            compressed: BTreeMap::new(),
            prev: None,
            size: 1,
        }
    }

    fn note(&mut self, num: u32) {
        self.size = self.size.max(num + 1);
    }

    /// `num 0 obj` BODY `endobj`.
    pub(crate) fn object(&mut self, num: u32, body: &[u8]) {
        self.object_gen(num, 0, body);
    }

    pub(crate) fn object_gen(&mut self, num: u32, generation: u16, body: &[u8]) {
        self.note(num);
        self.pending.insert(num, (self.out.len(), generation));
        self.out
            .extend_from_slice(format!("{num} {generation} obj\n").as_bytes());
        self.out.extend_from_slice(body);
        self.out.extend_from_slice(b"\nendobj\n");
    }

    /// A stream object: `<< /Length n DICT >>`, then DATA.
    pub(crate) fn stream(&mut self, num: u32, dict: &str, data: &[u8]) {
        let mut body = format!("<< /Length {} {dict} >>\nstream\n", data.len()).into_bytes();
        body.extend_from_slice(data);
        body.extend_from_slice(b"\nendstream");
        self.object(num, &body);
    }

    /// Raw bytes, not an object.
    pub(crate) fn raw(&mut self, bytes: &[u8]) {
        self.out.extend_from_slice(bytes);
    }

    /// An object stream `num` holding `objects` (number, body), Flate-compressed or not.
    pub(crate) fn object_stream(&mut self, num: u32, objects: &[(u32, &str)], compress: bool) {
        let mut header = String::new();
        let mut bodies = String::new();
        for (index, (object, body)) in objects.iter().enumerate() {
            write!(header, "{object} {} ", bodies.len()).unwrap();
            bodies.push_str(body);
            bodies.push('\n');
            self.note(*object);
            self.compressed
                .insert(*object, (num, u32::try_from(index).unwrap()));
        }
        let first = header.len();
        let data = format!("{header}{bodies}").into_bytes();
        let n = objects.len();
        if compress {
            self.stream(
                num,
                &format!("/Type /ObjStm /N {n} /First {first} /Filter /FlateDecode"),
                &deflate(&data),
            );
        } else {
            self.stream(num, &format!("/Type /ObjStm /N {n} /First {first}"), &data);
        }
    }

    fn prev_entry(&self) -> String {
        self.prev
            .map_or_else(String::new, |p| format!(" /Prev {p}"))
    }

    fn tail(&mut self, offset: usize) {
        self.out
            .extend_from_slice(format!("startxref\n{offset}\n%%EOF\n").as_bytes());
        self.prev = Some(offset);
    }

    /// Runs of consecutive numbers among `numbers`: (first, count).
    fn runs(numbers: impl Iterator<Item = u32>) -> Vec<(u32, u32)> {
        let mut runs: Vec<(u32, u32)> = Vec::new();
        for n in numbers {
            match runs.last_mut() {
                Some((first, count)) if *first + *count == n => *count += 1,
                _ => runs.push((n, 1)),
            }
        }
        runs
    }

    /// The `/Index` pairs of a cross-reference stream for `numbers`, as text.
    fn index(numbers: &[u32]) -> String {
        Self::runs(numbers.iter().copied())
            .iter()
            .fold(String::new(), |mut out, (first, count)| {
                write!(out, "{first} {count} ").unwrap();
                out
            })
    }

    /// A classic cross-reference table for the objects written since the last section
    /// (every number from 0 for the first section, free when not written: objects put in
    /// object streams are free in it), its trailer (`/Size`, `/Prev` and TRAILER),
    /// `startxref` and `%%EOF`.
    pub(crate) fn table(&mut self, trailer: &str) {
        self.table_of(trailer, self.prev.is_none());
    }

    /// As [`Writer::table`]; with `every`, every number from 0, else only object 0 (in a
    /// first section) and the objects written.
    fn table_of(&mut self, trailer: &str, every: bool) {
        let offset = self.out.len();
        let mut text = String::from("xref\n");
        if !every && self.prev.is_none() {
            text.push_str("0 1\n0000000000 65535 f \n");
        }
        if every {
            writeln!(text, "0 {}\n0000000000 65535 f ", self.size).unwrap();
            for num in 1..self.size {
                match self.pending.get(&num) {
                    Some((at, generation)) => {
                        writeln!(text, "{at:010} {generation:05} n ").unwrap();
                    }
                    None => text.push_str("0000000000 00000 f \n"),
                }
            }
        } else {
            for (first, count) in Self::runs(self.pending.keys().copied()) {
                writeln!(text, "{first} {count}").unwrap();
                for num in first..first + count {
                    let (at, generation) = self.pending[&num];
                    writeln!(text, "{at:010} {generation:05} n ").unwrap();
                }
            }
        }
        write!(
            text,
            "trailer\n<< /Size {}{} {trailer} >>\n",
            self.size,
            self.prev_entry()
        )
        .unwrap();
        self.out.extend_from_slice(text.as_bytes());
        self.pending.clear();
        self.tail(offset);
    }

    /// The rows (type, field 2, field 3) of a cross-reference stream for `numbers`.
    fn rows(&self, numbers: &[u32]) -> Vec<[u8; 7]> {
        numbers
            .iter()
            .map(|num| {
                let (kind, a, b) = if let Some((at, generation)) = self.pending.get(num) {
                    (1u8, u32::try_from(*at).unwrap(), *generation)
                } else if let Some((stream, index)) = self.compressed.get(num) {
                    (2, *stream, u16::try_from(*index).unwrap())
                } else {
                    (0, 0, if *num == 0 { 65535 } else { 0 })
                };
                let mut row = [0u8; 7];
                row[0] = kind;
                row[1..5].copy_from_slice(&a.to_be_bytes());
                row[5..7].copy_from_slice(&b.to_be_bytes());
                row
            })
            .collect()
    }

    /// A cross-reference stream object `num` for the objects written or compressed since
    /// the last section (every number from 0 for the first section), with /W [1 4 2],
    /// Flate-compressed, with the PNG Up predictor when `predictor`.
    pub(crate) fn xref_stream(&mut self, num: u32, trailer: &str, predictor: bool) {
        self.note(num);
        let offset = self.out.len();
        self.pending.insert(num, (offset, 0));
        let numbers: Vec<u32> = if self.prev.is_none() {
            (0..self.size).collect()
        } else {
            let mut all: Vec<u32> = self
                .pending
                .keys()
                .chain(self.compressed.keys())
                .copied()
                .collect();
            all.sort_unstable();
            all
        };
        let index = Self::index(&numbers);
        let rows = self.rows(&numbers);
        let dict = format!(
            "/Type /XRef /Size {} /W [1 4 2] /Index [{index}]{} {trailer}",
            self.size,
            self.prev_entry()
        );
        self.write_xref_stream(num, &dict, &rows, predictor);
        self.pending.clear();
        self.compressed.clear();
        self.tail(offset);
    }

    fn write_xref_stream(&mut self, num: u32, dict: &str, rows: &[[u8; 7]], predictor: bool) {
        if predictor {
            let mut data = Vec::new();
            let mut previous = [0u8; 7];
            for row in rows {
                data.push(2);
                for (byte, above) in row.iter().zip(previous) {
                    data.push(byte.wrapping_sub(above));
                }
                previous = *row;
            }
            self.stream(
                num,
                &format!("{dict} /Filter /FlateDecode /DecodeParms << /Predictor 12 /Columns 7 >>"),
                &deflate(&data),
            );
        } else {
            let data = rows.concat();
            self.stream(
                num,
                &format!("{dict} /Filter /FlateDecode"),
                &deflate(&data),
            );
        }
    }

    /// A cross-reference stream `num` for the compressed objects only, for `/XRefStm`;
    /// returns its offset.
    fn compressed_stream(&mut self, num: u32) -> usize {
        self.note(num);
        let compressed: Vec<u32> = self.compressed.keys().copied().collect();
        let index = Self::index(&compressed);
        let rows = self.rows(&compressed);
        let offset = self.out.len();
        let dict = format!(
            "/Type /XRef /Size {} /W [1 4 2] /Index [{index}]",
            self.size
        );
        self.write_xref_stream(num, &dict, &rows, true);
        self.compressed.clear();
        offset
    }

    /// A hybrid section (ISO 32000-2, 7.5.8.4): a cross-reference stream `num` for the
    /// compressed objects, then a classic table that does not list them, whose trailer
    /// points to the stream with `/XRefStm`. Must be the first section.
    pub(crate) fn hybrid(&mut self, num: u32, trailer: &str) {
        assert!(self.prev.is_none());
        let stream_offset = self.compressed_stream(num);
        self.table_of(&format!("{trailer} /XRefStm {stream_offset}"), false);
    }

    /// A hybrid file as word processors write it: a full classic table in which the
    /// compressed objects are free, then an update section with an empty table whose
    /// `/XRefStm` stream `num` lists them. Must be the first section.
    pub(crate) fn hybrid_update(&mut self, num: u32, trailer: &str) {
        assert!(self.prev.is_none());
        let compressed = self.compressed.clone();
        self.table(trailer);
        self.compressed = compressed;
        let stream_offset = self.compressed_stream(num);
        self.pending.clear();
        let offset = self.out.len();
        let text = format!(
            "xref\n0 0\ntrailer\n<< /Size {}{} {trailer} /XRefStm {stream_offset} >>\n",
            self.size,
            self.prev_entry()
        );
        self.out.extend_from_slice(text.as_bytes());
        self.tail(offset);
    }

    /// Continues a file already in `out`: its last section is at PREV, its object
    /// numbers are below SIZE.
    pub(crate) fn prev_from(&mut self, prev: usize, size: u32) {
        self.prev = Some(prev);
        self.size = self.size.max(size);
    }

    /// The offset of an object written since the last section.
    pub(crate) fn pending_offset(&self, num: u32) -> usize {
        self.pending[&num].0
    }

    /// Records objects as in object streams: (number, stream, index), for files whose
    /// object streams are written by hand.
    pub(crate) fn compressed_for_tests(&mut self, entries: &[(u32, u32, u32)]) {
        for (num, stream, index) in entries {
            self.note(*num);
            self.compressed.insert(*num, (*stream, *index));
        }
    }

    /// A first section, a classic table in which the objects of OVERRIDES are given the
    /// offsets listed there, then the file.
    pub(crate) fn finish_with_table_override(
        mut self,
        trailer: &str,
        overrides: &[(u32, usize)],
    ) -> Vec<u8> {
        for (num, offset) in overrides {
            self.note(*num);
            self.pending.insert(*num, (*offset, 0));
        }
        self.table(trailer);
        self.out
    }

    pub(crate) fn finish(self) -> Vec<u8> {
        self.out
    }
}

/// How a fixture is laid out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Layout {
    /// Every object uncompressed, a classic table.
    Table,
    /// Every object uncompressed, a cross-reference stream without predictor.
    XrefStream,
    /// Every object uncompressed, a cross-reference stream with the PNG Up predictor.
    XrefStreamPredictor,
    /// Catalog, pages and file specification in an object stream, a cross-reference
    /// stream with predictor.
    ObjectStreams,
    /// As `ObjectStreams`, with an uncompressed object stream and indirect `/Length`s.
    ObjectStreamsIndirectLength,
    /// Catalog, pages and file specification in an object stream, a hybrid section.
    Hybrid,
    /// Catalog, pages and file specification in an object stream, free in a full table,
    /// listed by the `/XRefStm` of an update section.
    HybridUpdate,
}

pub(crate) const LAYOUTS: [Layout; 7] = [
    Layout::Table,
    Layout::XrefStream,
    Layout::XrefStreamPredictor,
    Layout::ObjectStreams,
    Layout::ObjectStreamsIndirectLength,
    Layout::Hybrid,
    Layout::HybridUpdate,
];

/// A Factur-X-like PDF/A-3 file with one embedded file.
#[derive(Debug, Clone)]
pub(crate) struct Fixture<'a> {
    pub(crate) layout: Layout,
    pub(crate) name: &'a str,
    pub(crate) data: &'a [u8],
    pub(crate) compress: bool,
    /// The `/CheckSum` value as written (such as `<...>` or `(...)`), or none.
    pub(crate) checksum: Option<String>,
    pub(crate) relationship: &'a str,
    /// The XMP packet, or none (no `/Metadata`).
    pub(crate) xmp: Option<String>,
    /// List the file in the `/EmbeddedFiles` name tree.
    pub(crate) in_names: bool,
    /// List the file in the catalog's `/AF`.
    pub(crate) in_af: bool,
    /// More catalog entries, as text.
    pub(crate) catalog_extra: String,
}

impl Default for Fixture<'_> {
    fn default() -> Self {
        Self {
            layout: Layout::Table,
            name: "factur-x.xml",
            data: INVOICE,
            compress: true,
            checksum: Some(format!("<{INVOICE_MD5}>")),
            relationship: "/Data",
            xmp: Some(xmp("factur-x.xml", "EN 16931")),
            in_names: true,
            in_af: true,
            catalog_extra: String::new(),
        }
    }
}

/// A PDF literal string for ASCII text.
pub(crate) fn literal(text: &str) -> String {
    let mut out = String::from("(");
    for c in text.chars() {
        if matches!(c, '(' | ')' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out.push(')');
    out
}

/// An XMP packet with the PDF/A-3B identification and the Factur-X properties.
pub(crate) fn xmp(file_name: &str, level: &str) -> String {
    format!(
        "<?xpacket begin=\"\u{FEFF}\" id=\"W5M0MpCehiHzreSzNTczkc9d\"?>\n\
         <x:xmpmeta xmlns:x=\"adobe:ns:meta/\">\n\
         <rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\">\n\
         <rdf:Description rdf:about=\"\" xmlns:pdfaid=\"http://www.aiim.org/pdfa/ns/id/\">\n\
         <pdfaid:part>3</pdfaid:part><pdfaid:conformance>B</pdfaid:conformance>\n\
         </rdf:Description>\n\
         <rdf:Description rdf:about=\"\" xmlns:fx=\"urn:factur-x:pdfa:CrossIndustryDocument:invoice:1p0#\">\n\
         <fx:DocumentType>INVOICE</fx:DocumentType>\n\
         <fx:DocumentFileName>{file_name}</fx:DocumentFileName>\n\
         <fx:Version>1.0</fx:Version>\n\
         <fx:ConformanceLevel>{level}</fx:ConformanceLevel>\n\
         </rdf:Description>\n\
         </rdf:RDF>\n\
         </x:xmpmeta>\n\
         <?xpacket end=\"w\"?>"
    )
}

impl Fixture<'_> {
    /// Objects: 1 catalog, 2 pages, 3 page, 4 embedded file, 5 file specification,
    /// 6 metadata, 7 object stream, 8 cross-reference stream, 9 and 10 lengths.
    pub(crate) fn build(&self) -> Vec<u8> {
        let name = literal(self.name);
        let mut catalog = String::from("<< /Type /Catalog /Pages 2 0 R");
        if self.in_names {
            write!(
                catalog,
                " /Names << /EmbeddedFiles << /Names [{name} 5 0 R] >> >>"
            )
            .unwrap();
        }
        if self.in_af {
            catalog.push_str(" /AF [5 0 R]");
        }
        if self.xmp.is_some() {
            catalog.push_str(" /Metadata 6 0 R");
        }
        write!(catalog, " {} >>", self.catalog_extra).unwrap();
        let pages = "<< /Type /Pages /Count 1 /Kids [3 0 R] >>";
        let page = "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] >>";
        let spec = format!(
            "<< /Type /Filespec /F {name} /UF {name} /Desc (Invoice) /AFRelationship {} /EF << /F 4 0 R /UF 4 0 R >> >>",
            self.relationship
        );
        let mut params = format!("/Size {}", self.data.len());
        if let Some(checksum) = &self.checksum {
            write!(params, " /CheckSum {checksum}").unwrap();
        }
        let content = if self.compress {
            deflate(self.data)
        } else {
            self.data.to_vec()
        };
        let filter = if self.compress {
            " /Filter /FlateDecode"
        } else {
            ""
        };
        let file_dict = format!(
            "/Type /EmbeddedFile /Subtype /text#2Fxml{filter} /Params << {params} /ModDate (D:20260101000000+00'00') >>"
        );

        let mut w = Writer::new("1.7");
        let indirect = self.layout == Layout::ObjectStreamsIndirectLength;
        if indirect {
            // The embedded file with an indirect /Length (object 9).
            let mut body = format!("<< /Length 9 0 R {file_dict} >>\nstream\n").into_bytes();
            body.extend_from_slice(&content);
            body.extend_from_slice(b"\nendstream");
            w.object(4, &body);
        } else {
            w.stream(4, &file_dict, &content);
        }
        if let Some(xmp) = &self.xmp {
            w.stream(6, "/Type /Metadata /Subtype /XML", xmp.as_bytes());
        }
        let compressed = [
            (1, catalog.as_str()),
            (2, pages),
            (3, page),
            (5, spec.as_str()),
        ];
        match self.layout {
            Layout::Table | Layout::XrefStream | Layout::XrefStreamPredictor => {
                for (num, body) in compressed {
                    w.object(num, body.as_bytes());
                }
            }
            Layout::ObjectStreams | Layout::Hybrid | Layout::HybridUpdate => {
                w.object_stream(7, &compressed, true);
            }
            Layout::ObjectStreamsIndirectLength => {
                indirect_object_stream(&mut w, &compressed, content.len());
            }
        }
        let trailer = "/Root 1 0 R /ID [<00112233445566778899AABBCCDDEEFF> <00112233445566778899AABBCCDDEEFF>]";
        match self.layout {
            Layout::Table => w.table(trailer),
            Layout::XrefStream => w.xref_stream(8, trailer, false),
            Layout::XrefStreamPredictor
            | Layout::ObjectStreams
            | Layout::ObjectStreamsIndirectLength => w.xref_stream(8, trailer, true),
            Layout::Hybrid => w.hybrid(8, trailer),
            Layout::HybridUpdate => w.hybrid_update(8, trailer),
        }
        w.finish()
    }
}

/// Object stream 7 holding `objects` and object 9 (the embedded file's /Length,
/// `length`), its own /Length being object 10, uncompressed.
fn indirect_object_stream(w: &mut Writer, objects: &[(u32, &str)], length: usize) {
    let length = length.to_string();
    let mut objects = objects.to_vec();
    objects.push((9, length.as_str()));
    let mut header = String::new();
    let mut bodies = String::new();
    for (object, body) in &objects {
        write!(header, "{object} {} ", bodies.len()).unwrap();
        bodies.push_str(body);
        bodies.push('\n');
    }
    let first = header.len();
    let data = format!("{header}{bodies}");
    let body = format!(
        "<< /Type /ObjStm /N {} /First {first} /Length 10 0 R >>\nstream\n{data}\nendstream",
        objects.len()
    );
    w.object(7, body.as_bytes());
    w.object(10, data.len().to_string().as_bytes());
    for (index, (object, _)) in objects.iter().enumerate() {
        w.note(*object);
        w.compressed
            .insert(*object, (7, u32::try_from(index).unwrap()));
    }
}

/// A minimal PDF whose catalog is CATALOG (object 1), with EXTRA objects, classic table.
pub(crate) fn with_catalog(catalog: &str, extra: &[(u32, &[u8])]) -> Vec<u8> {
    let mut w = Writer::new("1.7");
    w.object(1, catalog.as_bytes());
    for (num, body) in extra {
        w.object(*num, body);
    }
    w.table("/Root 1 0 R");
    w.finish()
}

/// A file specification body naming NAME, embedding object STREAM.
pub(crate) fn filespec(name: &str, stream: u32) -> String {
    let name = literal(name);
    format!(
        "<< /Type /Filespec /F {name} /UF {name} /AFRelationship /Data /EF << /F {stream} 0 R >> >>"
    )
}

/// An embedded file stream body holding DATA, uncompressed.
pub(crate) fn embedded(data: &[u8]) -> Vec<u8> {
    let mut body = format!(
        "<< /Length {} /Type /EmbeddedFile /Subtype /text#2Fxml >>\nstream\n",
        data.len()
    )
    .into_bytes();
    body.extend_from_slice(data);
    body.extend_from_slice(b"\nendstream");
    body
}
