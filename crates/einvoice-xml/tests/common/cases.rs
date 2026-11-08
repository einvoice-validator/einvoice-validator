//! Hand-written documents: malformed ones that the reader must refuse as `roxmltree`
//! does, well-formed ones it must read as `roxmltree` does, and the documents where it
//! differs from `roxmltree` by design, with what it gives instead.

/// Malformed documents: the reader's error must be `roxmltree`'s (message, position).
pub(crate) const MALFORMED: &[(&str, &str)] = &[
    // The document and its structure.
    ("empty", ""),
    ("only spaces", " \n\t "),
    ("only a byte order mark", "\u{FEFF}"),
    ("only a declaration", "<?xml version=\"1.0\"?>"),
    ("only a comment", "<!-- c -->"),
    ("text at the top", "text"),
    ("text before the root", "x<a/>"),
    ("text after the root", "<a/>x"),
    ("two roots", "<a/><b/>"),
    ("cdata at the top", "<![CDATA[x]]><a/>"),
    ("cdata after the root", "<a/><![CDATA[x]]>"),
    ("end tag at the top", "</a>"),
    ("end tag after the root", "<a/></a>"),
    ("reference at the top", "&amp;<a/>"),
    ("lone less-than at the end", "<a></a><"),
    ("lone less-than in content", "<a><"),
    ("less-than and space", "<a>< b/></a>"),
    ("bang in content", "<a><!x></a>"),
    ("incomplete comment opener", "<a><!-x--></a>"),
    ("incomplete cdata opener", "<a><![CDAT[x]]></a>"),
    ("doctype in content", "<a><!DOCTYPE a></a>"),
    ("lowercase doctype", "<!doctype a><a/>"),
    ("doctype after the root", "<a/><!DOCTYPE a>"),
    ("element declaration", "<!ELEMENT a ANY><a/>"),
    // Unclosed and mismatched elements.
    ("unclosed root", "<a>"),
    ("unclosed child", "<a><b></a>"),
    ("unclosed with text", "<a>text"),
    ("unclosed nested", "<a><b><c/></b>"),
    ("mismatched end tag", "<a></b>"),
    ("mismatched nested", "<a>\n  <b>\n  </c>\n</a>"),
    (
        "end tag with another prefix",
        "<p:a xmlns:p='u' xmlns:q='u'></q:a>",
    ),
    ("end tag without the prefix", "<p:a xmlns:p='u'></a>"),
    ("end tag with a prefix", "<a xmlns:p='u'></p:a>"),
    ("start tag cut", "<a"),
    ("start tag cut after a space", "<a "),
    ("start tag cut after a name", "<a b"),
    ("start tag cut after equals", "<a b="),
    ("start tag cut in a value", "<a b='1"),
    ("start tag cut after a value", "<a b='1'"),
    ("child start tag cut", "<a><b c='1'"),
    ("empty tag cut", "<a/"),
    ("empty tag without greater-than", "<a/ >"),
    ("end tag cut", "<a></a"),
    ("end tag with attribute", "<a></a b='1'>"),
    ("end tag with junk", "<a></a x>"),
    ("end tag cut after name", "<a></a  "),
    // Names.
    ("name starting with a digit", "<1a/>"),
    ("name starting with a hyphen", "<-a/>"),
    ("name starting with a dot", "<a><.b/></a>"),
    ("name with two colons", "<a:b:c/>"),
    ("name ending with a colon", "<a:/>"),
    ("only a colon", "<:/>"),
    ("middle dot at the start", "<a><\u{B7}b/></a>"),
    ("combining mark at the start", "<\u{300}a/>"),
    ("attribute name starting with a digit", "<a 1b='1'/>"),
    ("attribute with two colons", "<a b:c:d='1'/>"),
    ("attribute without value", "<a b/>"),
    ("attribute without quotes", "<a b=1/>"),
    ("attribute without space", "<a b='1'c='2'/>"),
    ("attribute name then end", "<a b='1' c/>"),
    ("attribute with mismatched quotes", "<a b='1\"/>"),
    ("less-than in an attribute", "<a b='<'/>"),
    ("less-than in an attribute, double quotes", "<a b=\"x<y\"/>"),
    ("end tag name invalid", "<a></1a>"),
    ("pi without target", "<a><? x?></a>"),
    ("pi target with a digit first", "<a><?1x?></a>"),
    ("pi cut", "<a><?x"),
    ("pi cut in data", "<a><?x y"),
    // References.
    ("unknown entity", "<a>&foo;</a>"),
    ("unknown entity in an attribute", "<a b='&foo;'/>"),
    ("unknown entity with a colon", "<a>&a:b;</a>"),
    ("entity without semicolon", "<a>&amp</a>"),
    ("ampersand alone", "<a>a & b</a>"),
    ("ampersand in an attribute", "<a b='a & b'/>"),
    ("ampersand at the end", "<a>x&</a>"),
    ("character reference to zero", "<a>&#0;</a>"),
    ("character reference to a control", "<a>&#1;</a>"),
    ("character reference to FFFE", "<a>&#xFFFE;</a>"),
    ("character reference to FFFF", "<a>&#65535;</a>"),
    ("character reference with upper-case X", "<a>&#X41;</a>"),
    ("character reference without digits", "<a>&#;</a>"),
    ("hex reference without digits", "<a>&#x;</a>"),
    ("character reference without semicolon", "<a>&#65</a>"),
    (
        "character reference too large for u32",
        "<a>&#99999999999;</a>",
    ),
    ("character reference with a letter", "<a>&#6A;</a>"),
    ("control reference in an attribute", "<a b='&#2;'/>"),
    ("malformed then unknown", "<a>&#; &foo;</a>"),
    ("unknown then malformed", "<a>&foo; &#;</a>"),
    ("predefined entity in upper case", "<a>&AMP;</a>"),
    // Characters.
    ("control character in text", "<a>\u{1}</a>"),
    ("nul in text", "<a>\0</a>"),
    ("FFFE in text", "<a>\u{FFFE}</a>"),
    ("FFFF in text", "<a>x\u{FFFF}</a>"),
    ("control in an attribute", "<a b='\u{8}'/>"),
    ("control in a comment", "<a><!--\u{1}--></a>"),
    ("control in a pi", "<a><?x \u{1}?></a>"),
    ("control in cdata", "<a><![CDATA[\u{1f}]]></a>"),
    ("control in the declaration", "<?xml version='1\u{1}'?><a/>"),
    ("control at the top", "<a/>\u{1}"),
    ("control in a name", "<a\u{1}/>"),
    ("control after the root", "<a/>\n\u{7}"),
    ("control on a later line", "<a>\n\n   b\u{2}</a>"),
    ("byte order mark twice", "\u{FEFF}\u{FEFF}<a/>"),
    ("byte order mark in the middle", "<a/>\u{FEFF}"),
    ("non-character before the end", "<a>\u{e9}\u{FFFE}</a>"),
    ("control after non-ASCII", "<a>\u{20AC}\u{20AC}\u{1}</a>"),
    // Character data, comments, CDATA.
    ("cdata end in text", "<a>]]></a>"),
    ("cdata end after text", "<a>x ]]> y</a>"),
    ("cdata end in a later line", "<a>\nab]]>\ncd</a>"),
    ("double hyphen in a comment", "<a><!-- a -- b --></a>"),
    ("comment ending with a hyphen", "<a><!-- a ---></a>"),
    ("comment cut", "<a><!-- x"),
    ("comment cut at the top", "<!-- x"),
    ("comment with three hyphens", "<a><!-----></a>"),
    ("cdata cut", "<a><![CDATA[x"),
    ("cdata cut at the end", "<a><![CDATA[x]]"),
    // The XML declaration.
    ("declaration after a space", " <?xml version='1.0'?><a/>"),
    (
        "declaration after a comment",
        "<!-- c --><?xml version='1.0'?><a/>",
    ),
    ("declaration in content", "<a><?xml version='1.0'?></a>"),
    ("declaration after the root", "<a/><?xml version='1.0'?>"),
    (
        "declaration twice",
        "<?xml version='1.0'?><?xml version='1.0'?><a/>",
    ),
    (
        "declaration without version",
        "<?xml encoding='UTF-8'?><a/>",
    ),
    ("declaration without anything", "<?xml ?><a/>"),
    (
        "declaration with standalone first",
        "<?xml version='1.0' standalone='yes' encoding='UTF-8'?><a/>",
    ),
    (
        "declaration without space before encoding",
        "<?xml version='1.0'encoding='UTF-8'?><a/>",
    ),
    (
        "declaration without space after encoding",
        "<?xml version='1.0' encoding='UTF-8'standalone='no'?><a/>",
    ),
    (
        "declaration with an unknown attribute",
        "<?xml version='1.0' foo='x'?><a/>",
    ),
    (
        "declaration with version twice",
        "<?xml version='1.0' version='1.0'?><a/>",
    ),
    ("declaration without quotes", "<?xml version=1.0?><a/>"),
    ("declaration without equals", "<?xml version '1.0'?><a/>"),
    ("declaration cut", "<?xml version='1.0'"),
    ("declaration cut in the value", "<?xml version='1.0"),
    ("declaration with less-than", "<?xml version='1<'?><a/>"),
    (
        "declaration closed with greater-than",
        "<?xml version='1.0'><a/>",
    ),
    (
        "declaration with a question mark only",
        "<?xml version='1.0'?<a/>",
    ),
    // Document type declarations.
    ("doctype", "<!DOCTYPE a><a/>"),
    (
        "doctype with an internal subset",
        "<!DOCTYPE a [<!ENTITY e 'x'>]><a>&e;</a>",
    ),
    (
        "doctype after the declaration",
        "<?xml version='1.0'?>\n<!DOCTYPE a SYSTEM 'a.dtd'>\n<a/>",
    ),
    ("doctype after a comment", "<!-- c --><!DOCTYPE a><a/>"),
    ("malformed doctype", "<!DOCTYPEa><a/>"),
    // Namespaces.
    ("undeclared element prefix", "<p:a/>"),
    ("undeclared child prefix", "<a xmlns:p='u'><q:b/></a>"),
    ("undeclared attribute prefix", "<a p:b='1'/>"),
    ("prefix out of scope", "<a><b xmlns:p='u'/><p:c/></a>"),
    ("xmlns element prefix", "<xmlns:a/>"),
    ("xml element prefix", "<xml:a/>"),
    (
        "xml element prefix declared",
        "<xml:a xmlns:xml='http://www.w3.org/XML/1998/namespace'/>",
    ),
    ("xml prefix to another namespace", "<a xmlns:xml='urn:x'/>"),
    (
        "xml namespace to another prefix",
        "<a xmlns:p='http://www.w3.org/XML/1998/namespace'/>",
    ),
    (
        "xml namespace as the default",
        "<a xmlns='http://www.w3.org/XML/1998/namespace'/>",
    ),
    (
        "xmlns namespace declared",
        "<a xmlns:p='http://www.w3.org/2000/xmlns/'/>",
    ),
    (
        "xmlns namespace as the default",
        "<a xmlns='http://www.w3.org/2000/xmlns/'/>",
    ),
    (
        "xmlns namespace for xmlns",
        "<a xmlns:xmlns='http://www.w3.org/2000/xmlns/'/>",
    ),
    ("prefix declared twice", "<a xmlns:p='u' xmlns:p='v'/>"),
    (
        "prefix declared twice, same namespace",
        "<a xmlns:p='u' xmlns:p='u'/>",
    ),
    ("duplicate attribute", "<a b='1' b='2'/>"),
    ("duplicate attribute, three", "<a c='0' b='1' d='2' b='3'/>"),
    (
        "duplicate prefixed attribute",
        "<a xmlns:p='u' p:b='1' p:b='2'/>",
    ),
    (
        "duplicate attribute through two prefixes",
        "<a xmlns:p='u' xmlns:q='u' p:b='1' q:b='2'/>",
    ),
    (
        "duplicate attribute and unknown prefix",
        "<a b='1' b='2' q:c='3'/>",
    ),
    (
        "unknown prefix and duplicate attribute",
        "<a q:c='3' b='1' b='2'/>",
    ),
    (
        "duplicate xml attribute",
        "<a xml:lang='en' xml:lang='fr'/>",
    ),
    (
        "unknown attribute prefix and element prefix",
        "<p:a q:b='1'/>",
    ),
    (
        "many duplicate attributes",
        "<a a1='' a2='' a3='' a4='' a5='' a6='' a7='' a8='' a9='' a10='' a5=''/>",
    ),
    (
        "namespace error then tokenizer error",
        "<a xmlns:xml='urn:x' b/>",
    ),
    (
        "tokenizer error after duplicate attribute",
        "<a b='1' b='2' c/>",
    ),
    ("malformed reference in a namespace", "<a xmlns:p='&x'/>"),
    ("unknown entity in a namespace", "<a xmlns='&x;'/>"),
];

/// Well-formed documents: the reader's stream must be `roxmltree`'s.
pub(crate) const WELL_FORMED: &[(&str, &str)] = &[
    ("empty element", "<a/>"),
    ("element", "<a></a>"),
    (
        "declaration",
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<a/>",
    ),
    (
        "declaration with spaces",
        "<?xml   version = '1.0'   encoding = 'utf-8'  ?><a/>",
    ),
    ("declaration with version 1.1", "<?xml version='1.1'?><a/>"),
    ("byte order mark", "\u{FEFF}<?xml version='1.0'?><a/>"),
    ("byte order mark without declaration", "\u{FEFF}<a/>"),
    ("text", "<a>text</a>"),
    ("whitespace text", "<a>\n  <b/>\n  <c> </c>\n</a>"),
    (
        "prolog and epilog",
        "<!-- c1 -->\n<?pi data?>\n<a/>\n<!-- c2 --><?pi2?>\n",
    ),
    ("comments", "<a><!----><!-- x --><!-- - --><!-->--></a>"),
    (
        "processing instructions",
        "<a><?t?><?t data?><?t   spaced data ?><?xml-stylesheet href='x'?></a>",
    ),
    (
        "pi with colon-less target and question marks",
        "<a><?t a?b??></a>",
    ),
    ("cdata", "<a><![CDATA[<b>&amp;]]]]><![CDATA[>]]></a>"),
    ("empty cdata", "<a><![CDATA[]]></a>"),
    ("cdata and text joined", "<a>x<![CDATA[y]]>z</a>"),
    (
        "references",
        "<a>&lt;&gt;&amp;&quot;&apos;&#65;&#x42;&#x20AC;&#x10FFFF;</a>",
    ),
    (
        "references in attributes",
        "<a b='&lt;&#x9;&#10;&#13;&quot;' c=\"&apos;\"/>",
    ),
    (
        "attribute normalisation",
        "<a b='x\ty\nz\r\nw\rv' c='  spaces  '/>",
    ),
    ("line ends", "<a>x\r\ny\rz\r\n\r\n</a>"),
    ("line ends in cdata", "<a><![CDATA[x\r\ny\rz\r]]></a>"),
    (
        "character reference to carriage return",
        "<a>&#13;&#13;\n</a>",
    ),
    ("character reference then line end", "<a>&#65;\r\nB</a>"),
    ("greater-than in text", "<a>a > b ]] > c ]>]</a>"),
    ("quotes in attributes", "<a b=\"'\" c='\"'/>"),
    ("greater-than in an attribute", "<a b='>' c=']]>'/>"),
    ("default namespace", "<a xmlns='urn:a'><b/></a>"),
    (
        "default namespace undeclared",
        "<a xmlns='urn:a'><b xmlns=''><c/></b><d/></a>",
    ),
    ("empty default namespace at the root", "<a xmlns=''/>"),
    (
        "prefixes",
        "<p:a xmlns:p='urn:p' xmlns:q='urn:q' q:b='1' c='2'><q:c p:d='3'/></p:a>",
    ),
    (
        "prefix redeclared",
        "<p:a xmlns:p='urn:1'><p:b xmlns:p='urn:2'><p:c/></p:b><p:d/></p:a>",
    ),
    (
        "same attribute in two namespaces",
        "<a xmlns:p='urn:1' xmlns:q='urn:2' p:b='1' q:b='2' b='3'/>",
    ),
    (
        "xml attributes",
        "<a xml:lang='en' xml:space='preserve' xml:base='x'/>",
    ),
    (
        "xml prefix declared",
        "<a xmlns:xml='http://www.w3.org/XML/1998/namespace' xml:lang='en'/>",
    ),
    (
        "namespace with references",
        "<a xmlns='urn:&amp;x&#x20;y'/>",
    ),
    (
        "namespace with whitespace",
        "<a xmlns:p=' urn:p\t'><p:b/></a>",
    ),
    ("attribute named xmlns-like", "<a xmlnsx='1' xmlns-a='2'/>"),
    (
        "non-ASCII names",
        "<\u{e9}l\u{e9}ment \u{e0}ttr='\u{e9}'>\u{1F600}</\u{e9}l\u{e9}ment>",
    ),
    ("names with dots and hyphens", "<a.b-c_d><_x/></a.b-c_d>"),
    ("names with combining marks", "<a\u{300}\u{203F}b/>"),
    (
        "C1 and DEL characters",
        "<a b='\u{7F}\u{85}'>\u{80}\u{9F}\u{7F}</a>",
    ),
    (
        "non-characters other than FFFE",
        "<a>\u{FDD0}\u{1FFFE}\u{10FFFF}</a>",
    ),
    ("spaces in end tag", "<a></a \n\t>"),
    ("spaces in empty tag", "<a \n b='1' \t/>"),
    ("spaces around equals", "<a b \n= \t'1'/>"),
    ("deep", "<a><b><c><d><e><f/></e></d></c></b></a>"),
    ("mixed content", "<a>x<b>y</b>z<!--c-->w<?p q?>v</a>"),
    ("text with a lone carriage return at the end", "<a>x\r</a>"),
    ("whitespace after the root", "<a/>\n\n \t"),
    (
        "long attribute list",
        "<a a1='' a2='' a3='' a4='' a5='' a6='' a7='' a8='' a9='' a10='' a11='' a12='' a13='' a14='' a15='' a16='' a17='' a18=''/>",
    ),
    (
        "declaration with encoding names",
        "<?xml version='1.0' encoding='ISO-8859-1'?><a/>",
    ),
    (
        "pi named like xml but longer",
        "<a><?xml-model x?><?xmlfoo?></a>",
    ),
    ("standalone no", "<?xml version='1.0' standalone='no'?><a/>"),
];

/// Documents that `roxmltree` accepts (or refuses later) although XML 1.0 or
/// Namespaces in XML 1.0 forbids them: the reader's error (message and position).
pub(crate) const STRICT: &[(&str, &str, &str)] = &[
    (
        "surrogate reference",
        "<a>&#xD800;</a>",
        "the character reference at 1:4 is not a Unicode scalar value",
    ),
    (
        "reference above 10FFFF",
        "<a>&#x110000;</a>",
        "the character reference at 1:4 is not a Unicode scalar value",
    ),
    (
        "surrogate reference in an attribute",
        "<a b='x&#56320;'/>",
        "the character reference at 1:8 is not a Unicode scalar value",
    ),
    (
        "surrogate reference before an unknown entity",
        "<a>&#xD800;&foo;</a>",
        "unknown entity reference 'foo' at 1:12",
    ),
    (
        "pi named xml",
        "<a><?xml?></a>",
        "the processing instruction target at 1:6 is reserved or contains ':'",
    ),
    (
        "pi named XML at the start",
        "<?XML version='1.0'?><a/>",
        "the processing instruction target at 1:3 is reserved or contains ':'",
    ),
    (
        "pi named xMl",
        "<a><?xMl x?></a>",
        "the processing instruction target at 1:6 is reserved or contains ':'",
    ),
    (
        "declaration without data",
        "<?xml?><a/>",
        "the processing instruction target at 1:3 is reserved or contains ':'",
    ),
    (
        "declaration with a tab in content",
        "<a><?xml\tversion='1.0'?></a>",
        "the processing instruction target at 1:6 is reserved or contains ':'",
    ),
    (
        "pi target with a colon",
        "<a><?a:b x?></a>",
        "the processing instruction target at 1:6 is reserved or contains ':'",
    ),
    (
        "pi without whitespace",
        "<a><?abc?def?></a>",
        "expected a whitespace between the processing instruction target and its data at 1:9",
    ),
    (
        "pi with a quote after the target",
        "<a><?t\"x\"?></a>",
        "expected a whitespace between the processing instruction target and its data at 1:7",
    ),
    (
        "declaration with a misspelt version",
        "<?xml versionx='1.0'?><a/>",
        "invalid XML declaration at 1:7: unknown pseudo-attribute",
    ),
    (
        "declaration with a prefixed version",
        "<?xml version:a='1.0'?><a/>",
        "invalid XML declaration at 1:7: unknown pseudo-attribute",
    ),
    (
        "declaration with a bad version",
        "<?xml version='abc'?><a/>",
        "invalid XML declaration at 1:16: the version is not 1. followed by digits",
    ),
    (
        "declaration with version 2.0",
        "<?xml version='2.0'?><a/>",
        "invalid XML declaration at 1:16: the version is not 1. followed by digits",
    ),
    (
        "declaration with version 1.",
        "<?xml version=\"1.\"?><a/>",
        "invalid XML declaration at 1:16: the version is not 1. followed by digits",
    ),
    (
        "declaration with a bad encoding",
        "<?xml version='1.0' encoding='%%%'?><a/>",
        "invalid XML declaration at 1:31: the encoding is not an encoding name",
    ),
    (
        "declaration with an empty encoding",
        "<?xml version='1.0' encoding=''?><a/>",
        "invalid XML declaration at 1:31: the encoding is not an encoding name",
    ),
    (
        "declaration with a misspelt encoding",
        "<?xml version='1.0' encodingx='UTF-8'?><a/>",
        "invalid XML declaration at 1:21: unknown pseudo-attribute",
    ),
    (
        "declaration with a bad standalone",
        "<?xml version='1.0' standalone='maybe'?><a/>",
        "invalid XML declaration at 1:33: standalone is neither yes nor no",
    ),
    (
        "element with an empty prefix",
        "<:a/>",
        "a name with an empty prefix at 1:2",
    ),
    (
        "attribute with an empty prefix",
        "<a :b='1'/>",
        "a name with an empty prefix at 1:4",
    ),
    (
        "end tag with an empty prefix",
        "<a></:a>",
        "a name with an empty prefix at 1:6",
    ),
    (
        "empty-prefix attribute before an unknown prefix",
        "<a :b='1' q:c='2'/>",
        "an unknown namespace prefix 'q' at 1:11",
    ),
    (
        "prefix bound to nothing",
        "<a xmlns:p=''/>",
        "the prefix 'p' at 1:4 is bound to an empty namespace name",
    ),
    (
        "xmlns prefix declared",
        "<a xmlns:xmlns='urn:x'/>",
        "the 'xmlns' prefix is declared at 1:4, but it must not be",
    ),
    (
        "default namespace declared twice",
        "<a xmlns='urn:x' xmlns='urn:y'/>",
        "namespace declaration 'xmlns' at 1:18 is already defined",
    ),
    (
        "xml prefix declared twice",
        "<a xmlns:xml='http://www.w3.org/XML/1998/namespace' xmlns:xml='http://www.w3.org/XML/1998/namespace'/>",
        "namespace declaration 'xmlns:xml' at 1:53 is already defined",
    ),
    (
        "prefixed xmlns attribute",
        "<a p:xmlns='urn:x'/>",
        "attribute 'p:xmlns' at 1:4: a prefixed attribute named 'xmlns' is not supported",
    ),
    (
        "prefixed xmlns attribute with the prefix bound",
        "<a xmlns:p='urn:p' p:xmlns='urn:x'><b/></a>",
        "attribute 'p:xmlns' at 1:20: a prefixed attribute named 'xmlns' is not supported",
    ),
    (
        "strict error later than a roxmltree error in the same tag",
        "<a xmlns:p='' b='1' b='2'/>",
        "attribute 'b' at 1:21 is already defined",
    ),
];

/// Well-formed documents whose data the reader gives as XML 1.0 defines it, and
/// `roxmltree` otherwise: the reader's stream.
pub(crate) const DATA: &[(&str, &str, &[&str])] = &[
    (
        "carriage return after a reference, at the end",
        "<a>&#65;\r</a>",
        &["<{}a prefix=\"\"", "T\"A\\n\"", "</{}a>"],
    ),
    (
        "carriage return before a reference",
        "<a>\r&#65;</a>",
        &["<{}a prefix=\"\"", "T\"\\nA\"", "</{}a>"],
    ),
    (
        "carriage return between references",
        "<a>&amp;\r&amp;</a>",
        &["<{}a prefix=\"\"", "T\"&\\n&\"", "</{}a>"],
    ),
    (
        "two carriage returns before a reference",
        "<a>\r\r&#65;</a>",
        &["<{}a prefix=\"\"", "T\"\\n\\nA\"", "</{}a>"],
    ),
    (
        "declaration introduced by a tab",
        "<?xml\tversion='1.0'?><a/>",
        &["<{}a prefix=\"\"", "</{}a>"],
    ),
    (
        "declaration introduced by a line feed",
        "<?xml\nversion='1.0' encoding='UTF-8'?>\n<a/>",
        &["<{}a prefix=\"\"", "</{}a>"],
    ),
];

/// The raw comment and processing instruction values that `roxmltree` keeps with their
/// carriage returns, and the reader normalises.
pub(crate) const LINE_ENDS: &[(&str, &str, &str)] = &[
    ("comment", "<a><!-- x\r\ny\rz --></a>", " x\ny\nz "),
    ("pi", "<a><?t x\r\ny\rz?></a>", "x\ny\nz"),
];
