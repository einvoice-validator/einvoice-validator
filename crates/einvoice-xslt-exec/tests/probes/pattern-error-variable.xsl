<?xml version="1.0" encoding="UTF-8"?>
<!-- A global variable whose evaluation fails, referred to only in patterns: each reference evaluates it again (an error is not kept), each failure is a warning and the rule does not match. -->
<xsl:stylesheet version="2.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform" xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:f="urn:f">
  <xsl:output indent="no"/>
  <xsl:variable name="bad" select="concat('a', /r/x/@n)"/>
  <xsl:variable name="good" select="string(/r/@b)"/>
  <xsl:variable name="indirect" select="$bad = 'a1'"/>
  <xsl:template match="/"><out><xsl:apply-templates select="/r/*"/><xsl:value-of select="$good"/></out></xsl:template>
  <xsl:template match="x[$bad = 'a1']" priority="3">bad</xsl:template>
  <xsl:template match="y[$indirect]" priority="3">indirect</xsl:template>
  <xsl:template match="amounts[$good = 'two']" priority="3">good</xsl:template>
  <xsl:template match="*[$bad]" priority="2">bad-any</xsl:template>
  <!-- The second operand of and decides when the first fails: false, no error; true, the error -->
  <xsl:template match="codes[$bad = 'a1' and $good = 'z']" priority="4">and-false</xsl:template>
  <xsl:template match="text[$bad = 'a1' and $good = 'two']" priority="4">and-true</xsl:template>
  <xsl:template match="*">[<xsl:value-of select="name()"/>]</xsl:template>
</xsl:stylesheet>
