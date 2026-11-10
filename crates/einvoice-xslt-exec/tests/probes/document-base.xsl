<?xml version="1.0" encoding="UTF-8"?>
<!-- document(''): this stylesheet, its data elements; document() with two arguments: the
     URIs resolved against the base URI of the second (the source, this stylesheet). -->
<xsl:stylesheet version="2.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform" xmlns:data="urn:data">
  <data:item>from the stylesheet</data:item>
  <xsl:template match="/">
    <out>
      <self><xsl:value-of select="document('')//data:item"/></self>
      <self-templates><xsl:value-of select="count(document('')/*/xsl:template)"/></self-templates>
      <two-arguments><xsl:value-of select="document('ids.xml', /)/doc/@kind"/></two-arguments>
      <module><xsl:value-of select="count(document('modules/deeper.xsl', document(''))/*/xsl:template)"/></module>
      <nodes><xsl:value-of select="count(document(/r/@nope, /))"/></nodes>
    </out>
  </xsl:template>
</xsl:stylesheet>
