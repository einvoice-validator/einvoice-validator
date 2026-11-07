<?xml version="1.0" encoding="UTF-8"?>
<!-- Literal result elements in namespaces, computed attributes with prefixes. -->
<xsl:stylesheet version="2.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform" xmlns:q="urn:q" xmlns="urn:default">
  <xsl:output indent="no"/>
  <xsl:template match="/">
    <out a="1">
      <xsl:for-each select="/r/x[1]"><xsl:attribute name="{concat('d', @n)}" select="5"/></xsl:for-each>
      <q:inner q:b="2"><xsl:attribute name="q:c">3</xsl:attribute><xsl:attribute name="xml:lang">en</xsl:attribute><xsl:attribute name="plain">4</xsl:attribute></q:inner>
      <none xmlns=""><child/></none>
      <r:other xmlns:r="urn:r"><xsl:attribute name="r:x" xmlns:r="urn:r2">v</xsl:attribute><xsl:attribute name="s:y" xmlns:s="urn:r">w</xsl:attribute></r:other>
      <copied><xsl:sequence select="/r/@*"/><xsl:sequence select="/r/*[namespace-uri()]"/></copied>
    </out>
  </xsl:template>
</xsl:stylesheet>
