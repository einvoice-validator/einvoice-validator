<?xml version="1.0" encoding="UTF-8"?>
<!-- xsl:number with level single and multiple, with and without count. -->
<xsl:stylesheet version="2.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform">
  <xsl:output indent="no"/>
  <xsl:template match="/">
    <out><xsl:apply-templates select="//node() | //@*" mode="n"/><doc><xsl:number/>|<xsl:number count="*"/>|<xsl:number level="multiple"/></doc></out>
  </xsl:template>
  <xsl:template match="node() | @*" mode="n">(<xsl:number level="multiple" count="*"/>/<xsl:number count="node()"/>/<xsl:number/>/<xsl:number level="multiple"/>/<xsl:number count="x"/>/<xsl:number count="x[@n != '2']" level="multiple"/>)</xsl:template>
</xsl:stylesheet>
