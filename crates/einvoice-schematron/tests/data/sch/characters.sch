<?xml version="1.0" encoding="UTF-8"?>
<!-- Synthetic: characters the XML writer escapes or keeps: markup characters, quotes,
     tabs, line ends, carriage returns, C1 controls, non-ASCII and non-BMP characters, in
     text, in comments and in attribute values. -->
<schema xmlns="http://purl.oclc.org/dsdl/schematron" queryBinding="xslt2">
  <title>Characters: &lt; &gt; &amp; " ' é 😀</title>
  <ns prefix="a" uri="urn:example:a"/>
  <pattern id="C">
    <rule context="a:Item[@kind = &quot;x&quot; or @kind = 'y']">
      <assert id="C-1" test="string-length(.) &gt; 0 and . != '&lt;&amp;&gt;'">Markup: &lt; &amp; &gt; ]]&gt; "quoted" 'apostrophes'.</assert>
      <assert id="C-2" test="normalize-space(.) != '&#9;tab'">Tab:&#9;here, line end:
here, carriage return:&#13;here.</assert>
      <assert id="C-3" test="not(contains(., '&#x85;'))">C1 controls: &#x7F; &#x85; &#x9F;, after: &#xA0;.</assert>
      <assert id="C-4" test="not(contains(., '€'))">Non-ASCII: é ß € 中文, non-BMP: 😀 𝄞.</assert>
      <report id="C-5" test="@note" flag="a &amp; b">Attribute value in the message: <value-of select="@note"/>.</report>
    </rule>
  </pattern>
</schema>
