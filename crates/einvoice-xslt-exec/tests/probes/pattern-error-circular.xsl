<?xml version="1.0" encoding="UTF-8"?>
<!-- Error: a circular global variable met while matching a pattern is not recovered (XTDE0640). -->
<xsl:stylesheet version="2.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform" xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:f="urn:f">
  <xsl:output indent="no"/>
  <xsl:variable name="v"><xsl:apply-templates select="/r/x[1]" mode="m"/></xsl:variable>
  <xsl:template match="/"><out><xsl:value-of select="$v"/></out></xsl:template>
  <xsl:template match="x[string($v) = '']" mode="m" priority="2">circular</xsl:template>
  <xsl:template match="x" mode="m">x</xsl:template>
</xsl:stylesheet>
