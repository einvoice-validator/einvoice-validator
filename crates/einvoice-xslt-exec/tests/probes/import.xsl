<?xml version="1.0" encoding="UTF-8"?>
<!-- xsl:import (modules/imported.xsl, which imports modules/deeper.xsl): template rules
     ranked by import precedence before priority, xsl:next-match into a module of lower
     precedence, named templates and global variables overridden by those of a higher
     precedence (read from every module), document('') in an imported module. -->
<xsl:stylesheet version="2.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform">
  <xsl:import href="modules/imported.xsl"/>
  <xsl:param name="shared" select="'principal'"/>
  <xsl:template match="/">
    <out>
      <xsl:apply-templates select="/r/x[1]"/>
      <xsl:call-template name="named"/>
      <g><xsl:value-of select="$shared, $only-imported, $deep, $own"/></g>
      <xsl:apply-templates select="/r/y" mode="m"/>
      <xsl:apply-templates select="/r/codes" mode="m"/>
    </out>
  </xsl:template>
  <xsl:template match="x" priority="-5"><principal-low/></xsl:template>
  <xsl:template name="named"><principal-named/></xsl:template>
</xsl:stylesheet>
