# Test data of einvoice-schematron

- `sch/`: synthetic Schematron files written for these tests (`sch/parts/` holds the
  files they include), one feature set each: plain rules and messages (`basic`),
  abstract patterns, abstract rules and `let` (`abstract`), includes, `extends href` and
  phases (`include`), XSLT declarations, foreign namespaces, diagnostics and properties
  (`foreign`), escaped and dropped characters (`characters`), comments, processing
  instructions and the `xpath2` binding (`markup`), the smallest schema (`minimal`).
- `expected/`: for each, what ph-schematron 10.3.1 makes of it: the result of each step
  of the ISO Schematron skeleton (`NAME.step1.xml`, `NAME.step2.xml`, `NAME.step3.xml`,
  the last one the input of this crate), the DOM document of the last step
  (`NAME.dom`) and the compiled stylesheet as ph-schematron-maven-plugin writes it
  (`NAME.xslt`).
- `write/`: result trees for the DOM builder and the writer alone: `NAME.xml` is one, and
  `NAME.xsl` makes one when applied to itself (names the JDK's DOM refuses, nodes XSLT
  constructs); `written/` holds, for each, the DOM document (`NAME.dom`) and what the
  writer writes (`NAME.xslt`), or the code of the `DOMException` (`NAME.error`), and
  the result of `NAME.xsl` as Saxon serializes it (`NAME.result.xml`).
- `xml10-chars.txt`: the characters the XML writer of ph-commons 12.5.3 and the JDK's DOM
  take for invalid.
- `xerces-cases.txt` and `xerces-expected.txt`: operations on the JDK's DOM and their
  outcome.

Everything but `sch/`, `write/` and `xerces-cases.txt` is written by the `ph-schematron-oracle`
example (`cargo run -p einvoice-schematron --example ph-schematron-oracle -- --write`),
which runs ph-schematron-maven-plugin 10.3.1 and ph-schematron's own pipeline on the
JVM; without `--write` it checks these files.

The compiled stylesheets and the intermediate results contain text of the stylesheets
that produced them: the ISO Schematron skeleton of 2010, under the MIT licence (its
notice: "Copyright (c) 2004-2010 Rick Jellife and Academia Sinica Computing Centre,
Taiwan", with the permission notice of the MIT licence, `LICENSES/MIT.txt` of the crate
for its text), as packaged in ph-schematron, under the Apache License 2.0. These files
are test data of the repository and are not part of the published crate.
