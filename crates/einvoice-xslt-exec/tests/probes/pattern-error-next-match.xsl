<?xml version="1.0" encoding="UTF-8"?>
<!-- xsl:next-match: a rule whose pattern fails is skipped with a warning, as in xsl:apply-templates; a tie with a failing rule is no ambiguity. -->
<xsl:stylesheet version="2.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform" xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:f="urn:f">
  <xsl:output indent="no"/>
  <xsl:template match="/"><out><xsl:apply-templates select="/r/x | /r/y"/></out></xsl:template>
  <xsl:template match="x" priority="5">(5<xsl:next-match/>)</xsl:template>
  <xsl:template match="x[xs:integer(@n) + xs:integer(.) gt 0]" priority="4">(4<xsl:next-match/>)</xsl:template>
  <xsl:template match="x[normalize-space(../x) = '']" priority="3">(3<xsl:next-match/>)</xsl:template>
  <xsl:template match="x[@n = '2']" priority="2">(2a<xsl:next-match/>)</xsl:template>
  <xsl:template match="x[xs:date(@n) lt xs:date('2020-01-01')]" priority="2">(2b<xsl:next-match/>)</xsl:template>
  <xsl:template match="*" priority="1">(1)</xsl:template>
  <xsl:template match="y" priority="5">(y5<xsl:next-match/>)</xsl:template>
  <xsl:template match="y[xs:date(@z) gt xs:date('2000-01-01')]" priority="2">(y2a)</xsl:template>
  <xsl:template match="y[w]" priority="2">(y2b<xsl:next-match/>)</xsl:template>
</xsl:stylesheet>
