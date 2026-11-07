<?xml version="1.0" encoding="UTF-8"?>
<!-- Warning: two template rules of the same priority match one node; the last wins. -->
<xsl:stylesheet version="2.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform" xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:f="urn:f">
  <xsl:output indent="no"/>
  <xsl:template match="/"><out><xsl:apply-templates select="/r/x"/></out></xsl:template>
  <xsl:template match="x">first</xsl:template>
  <xsl:template match="*[@n]">second</xsl:template>
</xsl:stylesheet>
