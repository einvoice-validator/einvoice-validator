//! The pins of the fetched rule sets: a copy of the `name=value` block of each fetch
//! script of the repository (`scripts/fetch-<family>.sh`), in `pins/`, embedded. The
//! scripts are the single source of the pins; a test checks that each copy is the
//! block of its script, byte for byte, and that the URLs the scripts build from them
//! are the ones the catalogue builds.

/// The pins of one fetch script.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Pins {
    /// The script the block is copied from, relative to the repository root.
    pub(crate) script: &'static str,
    /// The block: one `name=value` line each.
    pub(crate) text: &'static str,
}

impl Pins {
    /// The value of the pin `name`.
    pub(crate) fn get(&self, name: &str) -> Result<&'static str, String> {
        self.text
            .lines()
            .find_map(|line| line.strip_prefix(name)?.strip_prefix('='))
            .filter(|v| !v.is_empty())
            .ok_or_else(|| format!("{}: no pin {name}", self.script))
    }

    /// `template` with each `{name}` replaced by the value of the pin `name`.
    pub(crate) fn expand(&self, template: &str) -> Result<String, String> {
        let mut out = String::with_capacity(template.len() + 32);
        let mut rest = template;
        while let Some(start) = rest.find('{') {
            out.push_str(&rest[..start]);
            let after = &rest[start + 1..];
            let end = after
                .find('}')
                .ok_or_else(|| format!("unterminated pin reference in {template:?}"))?;
            out.push_str(self.get(&after[..end])?);
            rest = &after[end + 1..];
        }
        out.push_str(rest);
        Ok(out)
    }
}

/// Factur-X: `scripts/fetch-facturx.sh`.
pub(crate) const FACTURX: Pins = Pins {
    script: "scripts/fetch-facturx.sh",
    text: include_str!("../../pins/fetch-facturx.pins"),
};

/// XRechnung: `scripts/fetch-xrechnung.sh`.
pub(crate) const XRECHNUNG: Pins = Pins {
    script: "scripts/fetch-xrechnung.sh",
    text: include_str!("../../pins/fetch-xrechnung.pins"),
};

/// The French rules: `scripts/fetch-fr-ctc.sh`.
pub(crate) const FR_CTC: Pins = Pins {
    script: "scripts/fetch-fr-ctc.sh",
    text: include_str!("../../pins/fetch-fr-ctc.pins"),
};

/// Peppol BIS Billing 3.0: `scripts/fetch-peppol.sh`.
pub(crate) const PEPPOL: Pins = Pins {
    script: "scripts/fetch-peppol.sh",
    text: include_str!("../../pins/fetch-peppol.pins"),
};

/// Every block, in the order of the catalogue.
#[cfg(test)]
pub(crate) const ALL: [Pins; 4] = [FACTURX, XRECHNUNG, FR_CTC, PEPPOL];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_line_is_a_pin() {
        for pins in ALL {
            assert!(pins.text.ends_with('\n'), "{}", pins.script);
            for line in pins.text.lines() {
                let (name, value) = line.split_once('=').unwrap();
                assert!(
                    !name.is_empty()
                        && name
                            .bytes()
                            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_'),
                    "{}: {line}",
                    pins.script
                );
                assert!(
                    !value.is_empty()
                        && value
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-'),
                    "{}: {line}",
                    pins.script
                );
            }
        }
    }

    #[test]
    fn expands_templates() {
        assert_ne!(FACTURX.get("facturx").unwrap(), "");
        assert_eq!(
            FACTURX.expand("a-{facturx}/{mustang}.jar").unwrap(),
            format!(
                "a-{}/{}.jar",
                FACTURX.get("facturx").unwrap(),
                FACTURX.get("mustang").unwrap()
            )
        );
        assert_eq!(FACTURX.expand("plain").unwrap(), "plain");
        assert!(
            FACTURX
                .expand("{nope}")
                .unwrap_err()
                .contains("no pin nope")
        );
        assert!(FACTURX.expand("{facturx").is_err());
        // A name that is the prefix of another is not that other.
        assert_eq!(
            FACTURX.get("z12_012").unwrap(),
            FACTURX
                .text
                .lines()
                .find_map(|l| l.strip_prefix("z12_012="))
                .unwrap()
        );
    }
}
