<?xml version="1.0" encoding="UTF-8"?>
<!-- Error: a local variable whose value depends on position() is evaluated where it is
     bound, even where nothing reads it. -->
<xsl:stylesheet version="2.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform" xmlns:xs="http://www.w3.org/2001/XMLSchema">
  <xsl:template match="/"><out><xsl:for-each select="/r/x"><xsl:variable name="m" select="xs:decimal(concat('x', position()))"/><xsl:if test="@n = '9' and ($m = 1 or $m = 2)">x</xsl:if></xsl:for-each></out></xsl:template>
</xsl:stylesheet>
