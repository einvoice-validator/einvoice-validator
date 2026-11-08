<?xml version="1.0" encoding="UTF-8"?>
<!-- Error: a template parameter default read in its first 20 evaluations is evaluated
     where it is bound from the 21st on (Saxon's learning). -->
<xsl:stylesheet version="2.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform" xmlns:xs="http://www.w3.org/2001/XMLSchema">
  <xsl:template match="/"><out><xsl:apply-templates select="for $i in 1 to 20 return /r/x[1]"/><xsl:apply-templates select="/r/y"/></out></xsl:template>
  <xsl:template match="x | y"><xsl:param name="p" select="xs:decimal(if (self::y) then 'x' else '1')"/><xsl:if test="self::x and ($p = 1 or $p = 2)">x</xsl:if>.</xsl:template>
</xsl:stylesheet>
