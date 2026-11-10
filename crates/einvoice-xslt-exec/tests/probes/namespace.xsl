<?xml version="1.0" encoding="UTF-8"?>
<!-- xsl:namespace: namespace nodes from `select` or content, on literal result elements and
     xsl:element, the same binding twice, the xml namespace, after an attribute, computed
     names, the default namespace on an element in it. -->
<xsl:stylesheet version="2.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform">
  <xsl:template match="/">
    <out>
      <a><xsl:namespace name="n1" select="'urn:n1'"/><xsl:namespace name="n2">urn:<xsl:value-of select="'n2'"/></xsl:namespace></a>
      <b xmlns:same="urn:same"><xsl:namespace name="same" select="'urn:same'"/><xsl:namespace name="same" select="'urn:same'"/></b>
      <c><xsl:namespace name="xml" select="'http://www.w3.org/XML/1998/namespace'"/></c>
      <d><xsl:attribute name="x">1</xsl:attribute><xsl:namespace name="after" select="'urn:after'"/></d>
      <e><xsl:for-each select="('p1', 'p2')"><xsl:namespace name="{.}" select="concat('urn:', .)"/></xsl:for-each></e>
      <f><xsl:element name="k"><xsl:namespace name="kk" select="'urn:kk'"/><inner/></xsl:element></f>
      <g><xsl:element name="d" namespace="urn:default"><xsl:namespace name="" select="'urn:default'"/></xsl:element></g>
      <h><xsl:namespace name=" spaced " select="'urn:spaced'"/></h>
    </out>
  </xsl:template>
</xsl:stylesheet>
