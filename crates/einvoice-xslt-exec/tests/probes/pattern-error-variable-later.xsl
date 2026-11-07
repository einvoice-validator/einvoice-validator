<?xml version="1.0" encoding="UTF-8"?>
<!-- Error: a global variable whose evaluation failed in a pattern (a warning) is evaluated again when referred to outside a pattern, where the error stops the transformation. -->
<xsl:stylesheet version="2.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform" xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:f="urn:f">
  <xsl:output indent="no"/>
  <xsl:variable name="bad" select="concat('a', /r/x/@n)"/>
  <xsl:template match="/"><out><xsl:apply-templates select="/r/x"/><xsl:value-of select="$bad"/></out></xsl:template>
  <xsl:template match="x[$bad = 'a1']" priority="3">bad</xsl:template>
  <xsl:template match="*">[<xsl:value-of select="name()"/>]</xsl:template>
</xsl:stylesheet>
