<?xml version="1.0" encoding="UTF-8"?>
<!-- Error: global variables that depend on each other. -->
<xsl:stylesheet version="2.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform" xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:f="urn:f">
  <xsl:variable name="a" select="$b + 1"/>
  <xsl:variable name="b" select="$a + 1"/>
  <xsl:template match="/"><out><xsl:value-of select="$a"/></out></xsl:template>
</xsl:stylesheet>
