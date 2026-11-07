# Canonical SVRL, version 1

The golden files (`corpus/<rule set>/golden/<case id>.svrl.txt`) hold the SVRL report of the
reference engine in a canonical, line-oriented form, so that two engines can be compared with a
plain text diff. It keeps every SVRL element, attribute, text and comment of the report, in
document order. The implementation is `xtask/src/svrl.rs`.

## Lines

The file is UTF-8 with LF line ends. The first line is

```text
svrl-canonical 1
```

Then one line per SVRL element and per comment, in document order:

```text
<indent><kind>( <name>="<value>")*( "<text>")?
```

- `kind` is the local name of the SVRL element (`schematron-output`,
  `ns-prefix-in-attribute-values`, `active-pattern`, `fired-rule`, `failed-assert`,
  `successful-report`, `text`, ...) or `comment` for a comment.
- `indent` is two spaces per level. The root `schematron-output` and its children are at level
  0; the children of those (the `text` of a `failed-assert`, ...) at level 1, and so on.
- Attributes are sorted by name. A namespaced attribute is written `xml:<name>` in the XML
  namespace and `{<namespace URI>}<name>` otherwise.
- Attribute values are verbatim, as the XML parser reports them, with one exception:
  in `active-pattern/@documents`, the URI of the validated document is written `{document}`.
  Any other URI there is an error.
- Elements that hold text (`text`, `diagnostic-reference`) end with their text, always present,
  even when empty (`""`). Comments are written `comment "<text>"`. Both are whitespace
  normalised as by XPath `normalize-space`: leading and trailing space, tab, CR and LF are
  removed and inner runs become one space. Other characters, the no-break space included, are
  kept.

Example:

```text
svrl-canonical 1
schematron-output schemaVersion="" title="EN16931  model bound to UBL"
comment "\u{a0} \u{a0} \u{a0}"
ns-prefix-in-attribute-values prefix="cbc" uri="urn:oasis:names:specification:ubl:schema:xsd:CommonBasicComponents-2"
active-pattern documents="{document}" id="UBL-model" name="UBL-model"
fired-rule context="/ubl:Invoice | /cn:CreditNote"
failed-assert flag="fatal" id="BR-02" location="/*:Invoice[namespace-uri()='urn:oasis:names:specification:ubl:schema:xsd:Invoice-2'][1]" test="normalize-space(cbc:ID) != ''"
  text "[BR-02]-An Invoice shall have an Invoice number (BT-1)."
```

## Quoting

Values and texts are between double quotes. Inside them, `\` is written `\\`, `"` is `\"`,
LF, CR and tab are `\n`, `\r` and `\t`. Other control characters, whitespace other than the
plain space (U+00A0 no-break space, ...), zero-width characters (U+200B to U+200F, U+2060),
U+FEFF and the soft hyphen U+00AD are written `\u{<lower-case hex>}`, so that nothing invisible
hides in a diff. Every other character is written as is.

## What is left out

Only what carries no information about the validation: the XML declaration, the namespace
declarations of the report (the SVRL namespace is implied; the rule set's prefixes are listed
by the `ns-prefix-in-attribute-values` lines), whitespace-only text between elements, and
attribute order.

## What is refused

A report that this form cannot represent faithfully is an error, never silently trimmed: an
element outside the SVRL namespace or not listed above, markup inside a `text`, text directly
inside a container element, a processing instruction, or a `documents` URI other than the
validated document.

## Equivalence

Two engines agree on a document when their canonical files are identical: the same active
patterns, fired rules, failed asserts and successful reports, in the same order, with the same
ids, flags, contexts, tests, locations and message texts.
