<?xml version="1.0" encoding="UTF-8"?>
<!-- xsl:attribute with `namespace`: the prefix of the name kept, an empty namespace (no
     prefix), the prefix of a namespace the element has, the xml namespace. -->
<xsl:stylesheet version="2.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform" xmlns:p="urn:p">
  <xsl:template match="/">
    <out><xsl:attribute name="q:a" namespace="urn:q1">1</xsl:attribute><xsl:attribute name="p:b" namespace="">2</xsl:attribute><xsl:attribute name="p:c" namespace="urn:p">3</xsl:attribute><xsl:attribute name="p:d" namespace="urn:other">4</xsl:attribute><xsl:attribute name="xml:lang">en</xsl:attribute></out>
  </xsl:template>
</xsl:stylesheet>
