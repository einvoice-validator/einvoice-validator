<?xml version="1.0" encoding="UTF-8"?>
<!-- Error: xsl:next-match evaluates its parameters before the next rule runs, even where
     nothing reads them. -->
<xsl:stylesheet version="2.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform" xmlns:xs="http://www.w3.org/2001/XMLSchema">
  <xsl:template match="/"><out><xsl:apply-templates select="/r"/></out></xsl:template>
  <xsl:template match="r"><xsl:next-match><xsl:with-param name="p" select="xs:decimal(@b)"/></xsl:next-match></xsl:template>
  <xsl:template match="*" priority="-1"><xsl:param name="p"/><xsl:if test="@a = '9' and $p = 1">x</xsl:if>.</xsl:template>
</xsl:stylesheet>
