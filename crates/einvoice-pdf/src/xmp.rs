//! The XMP metadata packet (ISO 16684-1): the PDF/A identification (`pdfaid:part`,
//! `pdfaid:conformance`) and the Factur-X / `ZUGFeRD` / Order-X extension schema
//! properties, written as elements or as attributes of `rdf:Description`.

use roxmltree::{Document, Node, ParsingOptions};

use crate::error::{Error, Result};
use crate::{InvoiceMetadata, Metadata};

/// The namespace of the PDF/A identification schema.
const PDFA_ID: &str = "http://www.aiim.org/pdfa/ns/id/";

/// Whether `uri` is the namespace of a Factur-X, `ZUGFeRD` or Order-X extension schema:
/// `urn:<scheme>:pdfa:CrossIndustryDocument:<rest>`, such as
/// `urn:factur-x:pdfa:CrossIndustryDocument:invoice:1p0#`.
pub(crate) fn is_invoice_namespace(uri: &str) -> bool {
    uri.strip_prefix("urn:")
        .and_then(|rest| rest.split_once(':'))
        .is_some_and(|(scheme, rest)| {
            !scheme.is_empty() && rest.starts_with("pdfa:CrossIndustryDocument:")
        })
}

#[derive(Default)]
struct Fields {
    namespace: Option<String>,
    document_type: Option<String>,
    document_file_name: Option<String>,
    version: Option<String>,
    conformance_level: Option<String>,
    pdfa_part: Option<String>,
    pdfa_conformance: Option<String>,
}

fn set(slot: &mut Option<String>, value: &str, what: &str) -> Result<()> {
    match slot {
        Some(existing) if existing != value => Err(Error::Metadata(format!(
            "two values for {what}: {existing:?} and {value:?}"
        ))),
        Some(_) => Ok(()),
        None => {
            *slot = Some(value.to_owned());
            Ok(())
        }
    }
}

impl Fields {
    fn property(&mut self, namespace: &str, name: &str, value: &str) -> Result<()> {
        if namespace == PDFA_ID {
            return match name {
                "part" => set(&mut self.pdfa_part, value, "pdfaid:part"),
                "conformance" => set(&mut self.pdfa_conformance, value, "pdfaid:conformance"),
                _ => Ok(()),
            };
        }
        if !is_invoice_namespace(namespace) {
            return Ok(());
        }
        let slot = match name {
            "DocumentType" => &mut self.document_type,
            "DocumentFileName" => &mut self.document_file_name,
            "Version" => &mut self.version,
            "ConformanceLevel" => &mut self.conformance_level,
            _ => return Ok(()),
        };
        set(slot, value, name)?;
        match &self.namespace {
            Some(known) if known != namespace => Err(Error::Metadata(format!(
                "properties in two invoice extension schemas: {known} and {namespace}"
            ))),
            Some(_) => Ok(()),
            None => {
                self.namespace = Some(namespace.to_owned());
                Ok(())
            }
        }
    }
}

/// The text of a simple property element.
fn simple_value<'a>(node: Node<'a, '_>, name: &str) -> Result<&'a str> {
    if node.children().any(|child| child.is_element()) {
        return Err(Error::Metadata(format!(
            "{name} is not a simple text property"
        )));
    }
    Ok(node.text().unwrap_or(""))
}

/// Parses the packet; `max_nodes` bounds the XML tree.
pub(crate) fn parse(bytes: Vec<u8>, max_nodes: u32) -> Result<Metadata> {
    let xmp = String::from_utf8(bytes)
        .map_err(|_| Error::Metadata("the metadata stream is not UTF-8".to_owned()))?;
    let options = ParsingOptions {
        allow_dtd: false,
        nodes_limit: max_nodes,
        ..ParsingOptions::default()
    };
    let mut fields = Fields::default();
    {
        let document = Document::parse_with_options(&xmp, options)
            .map_err(|e| Error::Metadata(format!("not well-formed XML: {e}")))?;
        for node in document.descendants().filter(Node::is_element) {
            if let Some(namespace) = node.tag_name().namespace() {
                let name = node.tag_name().name();
                if namespace == PDFA_ID || is_invoice_namespace(namespace) {
                    let value = simple_value(node, name)?;
                    fields.property(namespace, name, value)?;
                }
            }
            for attribute in node.attributes() {
                if let Some(namespace) = attribute.namespace() {
                    fields.property(namespace, attribute.name(), attribute.value())?;
                }
            }
        }
    }
    let invoice = fields.namespace.map(|namespace| InvoiceMetadata {
        namespace,
        document_type: fields.document_type,
        document_file_name: fields.document_file_name,
        version: fields.version,
        conformance_level: fields.conformance_level,
    });
    Ok(Metadata {
        xmp,
        pdfa_part: fields.pdfa_part,
        pdfa_conformance: fields.pdfa_conformance,
        invoice,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const RDF: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#";

    fn packet(body: &str) -> Vec<u8> {
        format!(
            "<?xpacket begin=\"\u{FEFF}\" id=\"W5M0MpCehiHzreSzNTczkc9d\"?>\
             <x:xmpmeta xmlns:x=\"adobe:ns:meta/\"><rdf:RDF xmlns:rdf=\"{RDF}\">{body}</rdf:RDF>\
             </x:xmpmeta><?xpacket end=\"w\"?>"
        )
        .into_bytes()
    }

    #[test]
    fn namespaces() {
        for uri in [
            "urn:factur-x:pdfa:CrossIndustryDocument:invoice:1p0#",
            "urn:zugferd:pdfa:CrossIndustryDocument:invoice:2p0#",
            "urn:ferd:pdfa:CrossIndustryDocument:invoice:1p0#",
            "urn:factur-x:pdfa:CrossIndustryDocument:1p0#",
        ] {
            assert!(is_invoice_namespace(uri), "{uri}");
        }
        for uri in [
            "http://www.aiim.org/pdfa/ns/id/",
            "urn::pdfa:CrossIndustryDocument:x",
            "urn:factur-x:CrossIndustryDocument:invoice:1p0#",
            "factur-x:pdfa:CrossIndustryDocument:invoice:1p0#",
        ] {
            assert!(!is_invoice_namespace(uri), "{uri}");
        }
    }

    #[test]
    fn elements_and_attributes() {
        let body = "<rdf:Description rdf:about=\"\" xmlns:pdfaid=\"http://www.aiim.org/pdfa/ns/id/\">\
                    <pdfaid:part>3</pdfaid:part><pdfaid:conformance>B</pdfaid:conformance></rdf:Description>\
                    <rdf:Description rdf:about=\"\" xmlns:fx=\"urn:factur-x:pdfa:CrossIndustryDocument:invoice:1p0#\" \
                    fx:Version=\"1.0\"><fx:DocumentType>INVOICE</fx:DocumentType>\
                    <fx:DocumentFileName>factur-x.xml</fx:DocumentFileName>\
                    <fx:ConformanceLevel>EN 16931</fx:ConformanceLevel></rdf:Description>";
        let m = parse(packet(body), 10_000).unwrap();
        assert_eq!(m.pdfa_part.as_deref(), Some("3"));
        assert_eq!(m.pdfa_conformance.as_deref(), Some("B"));
        let i = m.invoice.unwrap();
        assert_eq!(
            i.namespace,
            "urn:factur-x:pdfa:CrossIndustryDocument:invoice:1p0#"
        );
        assert_eq!(i.document_type.as_deref(), Some("INVOICE"));
        assert_eq!(i.document_file_name.as_deref(), Some("factur-x.xml"));
        assert_eq!(i.version.as_deref(), Some("1.0"));
        assert_eq!(i.conformance_level.as_deref(), Some("EN 16931"));
    }

    #[test]
    fn no_invoice_schema() {
        let m = parse(packet(""), 10_000).unwrap();
        assert!(m.invoice.is_none());
        assert!(m.pdfa_part.is_none());
    }

    #[test]
    fn refusals() {
        let conflict = "<rdf:Description xmlns:fx=\"urn:factur-x:pdfa:CrossIndustryDocument:invoice:1p0#\" \
                        fx:Version=\"1.0\"><fx:Version>2.0</fx:Version></rdf:Description>";
        assert!(matches!(
            parse(packet(conflict), 10_000),
            Err(Error::Metadata(_))
        ));
        let two = "<rdf:Description xmlns:fx=\"urn:factur-x:pdfa:CrossIndustryDocument:invoice:1p0#\" \
                   xmlns:zf=\"urn:ferd:pdfa:CrossIndustryDocument:invoice:1p0#\" \
                   fx:Version=\"1.0\" zf:DocumentType=\"INVOICE\"/>";
        assert!(matches!(
            parse(packet(two), 10_000),
            Err(Error::Metadata(_))
        ));
        let structured = "<rdf:Description xmlns:fx=\"urn:factur-x:pdfa:CrossIndustryDocument:invoice:1p0#\">\
                          <fx:Version><rdf:Alt/></fx:Version></rdf:Description>";
        assert!(matches!(
            parse(packet(structured), 10_000),
            Err(Error::Metadata(_))
        ));
        assert!(matches!(
            parse(b"<a>".to_vec(), 10_000),
            Err(Error::Metadata(_))
        ));
        assert!(matches!(
            parse(vec![0xFF, 0xFE], 10_000),
            Err(Error::Metadata(_))
        ));
        assert!(matches!(parse(packet(""), 3), Err(Error::Metadata(_))));
        let dtd = "<!DOCTYPE a [<!ENTITY e \"x\">]><a>&e;</a>";
        assert!(matches!(
            parse(dtd.as_bytes().to_vec(), 10_000),
            Err(Error::Metadata(_))
        ));
    }
}
