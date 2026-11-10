<?xml version="1.0" encoding="UTF-8"?>
<!-- Imported by imported.xsl. -->
<xsl:stylesheet version="2.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform">
  <xsl:variable name="deep" select="'deep'"/>
  <xsl:template match="y" mode="m" priority="100"><deeper-m/></xsl:template>
  <xsl:template match="*" mode="m"><deeper-any name="{name()}"/></xsl:template>
</xsl:stylesheet>
