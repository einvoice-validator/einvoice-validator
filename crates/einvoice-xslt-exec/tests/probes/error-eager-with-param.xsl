<?xml version="1.0" encoding="UTF-8"?>
<!-- Error: xsl:apply-templates evaluates its parameters before applying templates, even
     where nothing reads them. -->
<xsl:stylesheet version="2.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform" xmlns:xs="http://www.w3.org/2001/XMLSchema">
  <xsl:template match="/"><out><xsl:apply-templates select="/r"><xsl:with-param name="p" select="xs:decimal(/r/@b)"/></xsl:apply-templates></out></xsl:template>
  <xsl:template match="r"><xsl:param name="p"/><xsl:if test="@a = '9' and $p = 1">x</xsl:if>.</xsl:template>
</xsl:stylesheet>
