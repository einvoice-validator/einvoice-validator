<?xml version="1.0" encoding="UTF-8"?>
<!-- Synthetic: the plain features of a Schematron schema compiled for XSLT 2.0. -->
<schema xmlns="http://purl.oclc.org/dsdl/schematron" queryBinding="xslt2" schemaVersion="1.0" id="basic" xml:lang="en">
  <title>Basic schema</title>
  <p>A paragraph of the schema.</p>
  <ns prefix="a" uri="urn:example:a"/>
  <ns prefix="b" uri="urn:example:b"/>
  <ns prefix="xs" uri="http://www.w3.org/2001/XMLSchema"/>
  <pattern id="P1">
    <title>First pattern</title>
    <p>A paragraph of the pattern.</p>
    <rule context="/a:Order">
      <assert id="R-1" flag="fatal" test="a:Id" see="https://example.org/rules#R-1" icon="https://example.org/r.png" fpi="+//IDN example.org//R-1//EN">An order has an identifier.</assert>
      <assert id="R-2" flag="warning" role="warn" test="count(a:Line) &gt;= 1">An order has <emph>at least</emph> one line, not <value-of select="count(a:Line)"/>.</assert>
      <report id="R-3" flag="information" test="a:Note" subject="a:Note">The order <name/> has a note: <value-of select="a:Note"/>.</report>
    </rule>
    <rule context="a:Line">
      <assert id="R-4" flag="fatal" test="xs:decimal(a:Amount) &gt;= 0 and string-length(a:Id) &lt; 10">A line amount is not negative &amp; its id is short.</assert>
    </rule>
  </pattern>
  <pattern id="P2">
    <rule context="b:*">
      <report id="R-5" test="true()">Element <name path="."/> of <span class="x">namespace</span> b, <dir value="ltr">left to right</dir>.</report>
    </rule>
  </pattern>
</schema>
