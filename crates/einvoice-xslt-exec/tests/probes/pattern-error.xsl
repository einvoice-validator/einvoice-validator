<?xml version="1.0" encoding="UTF-8"?>
<!-- Dynamic errors while matching a pattern: the rule does not match, the error is a warning, and the search goes on. -->
<xsl:stylesheet version="2.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform" xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:f="urn:f" xmlns:p="urn:p">
  <xsl:output indent="no"/>
  <xsl:template match="/"><out><xsl:apply-templates select="/r/*"/></out></xsl:template>
  <!-- XPTY0004: two items where one is allowed (y has two text children) -->
  <xsl:template match="y[normalize-space(text()) = 't2']" priority="5">y-type</xsl:template>
  <!-- FORG0001 on every element: xs:decimal('') -->
  <xsl:template match="*[xs:decimal(string(@b)) gt 0]" priority="4">decimal</xsl:template>
  <!-- FORG0001 on every x, with three different values -->
  <xsl:template match="x[xs:decimal(.) = 1]" priority="3">x-decimal</xsl:template>
  <!-- FOAR0001 in a predicate that may be positional (evaluated from the parent) -->
  <xsl:template match="amounts[count(*) idiv 0]" priority="2">idiv</xsl:template>
  <!-- A union pattern with an explicit priority is one rule: the first alternative fails, the second is not tried -->
  <xsl:template match="codes[xs:integer(.) = 1] | codes" priority="1.5">codes-union</xsl:template>
  <!-- A union pattern without priority is a rule per alternative: the first fails, the second matches -->
  <xsl:template match="text[xs:integer(.) = 1] | text">text-split</xsl:template>
  <!-- A tie: the rule declared last matches; the other fails, so there is no ambiguity -->
  <xsl:template match="p:x[xs:integer(.) = 5]" priority="6">px-integer</xsl:template>
  <xsl:template match="p:x" priority="6">px</xsl:template>
  <xsl:template match="*" priority="-1">[<xsl:value-of select="name()"/>]</xsl:template>
</xsl:stylesheet>
