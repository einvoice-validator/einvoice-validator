<?xml version="1.0" encoding="UTF-8"?>
<!-- Imported by import.xsl; imports deeper.xsl. -->
<xsl:stylesheet version="2.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform">
  <xsl:import href="deeper.xsl"/>
  <xsl:param name="shared" select="'imported'"/>
  <xsl:variable name="only-imported" select="concat('imported sees ', $shared)"/>
  <xsl:variable name="own" select="count(document('')/*/xsl:template)"/>
  <xsl:template match="x" priority="10"><imported-high/></xsl:template>
  <xsl:template name="named"><imported-named/></xsl:template>
  <xsl:template match="y" mode="m"><imported-m><xsl:next-match/></imported-m></xsl:template>
</xsl:stylesheet>
