<?xml version="1.0" encoding="UTF-8"?>
<!-- Error: xsl:next-match where there is no current template rule. -->
<xsl:stylesheet version="2.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform" xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:f="urn:f">
  <xsl:template match="/"><out><xsl:apply-templates select="/r"/></out></xsl:template>
  <xsl:template match="r"><xsl:for-each select="x"><xsl:next-match/></xsl:for-each></xsl:template>
</xsl:stylesheet>
