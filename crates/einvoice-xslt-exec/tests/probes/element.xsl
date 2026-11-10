<?xml version="1.0" encoding="UTF-8"?>
<!-- xsl:element: a name in no namespace, a prefix of the instruction's namespaces, the
     default namespace of the instruction, the `namespace` attribute with and without a
     prefix in the name and empty, a name computed, attributes and content. -->
<xsl:stylesheet version="2.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform" xmlns:q="urn:q">
  <xsl:template match="/">
    <out>
      <e1><xsl:element name="plain"/></e1>
      <e2><xsl:element name="q:pre"/></e2>
      <e3><xsl:element name="n" namespace="urn:n"/></e3>
      <e4><xsl:element name="z:n" namespace="urn:z"/></e4>
      <e5><xsl:element name="q:n" namespace=""/></e5>
      <e6 xmlns="urn:dflt"><xsl:element name="inherits"/></e6>
      <e7><xsl:element name="{concat('a', 'b')}" namespace="{'urn:ab'}"><xsl:attribute name="x">1</xsl:attribute>t<xsl:element name="child"/></xsl:element></e7>
      <e8><xsl:element name=" spaced "/></e8>
    </out>
  </xsl:template>
</xsl:stylesheet>
