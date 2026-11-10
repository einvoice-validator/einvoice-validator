<?xml version="1.0" encoding="UTF-8"?>
<!-- xsl:processing-instruction: content, `select` (joined with spaces), leading whitespace
     removed, `?>` written `? >`, a computed name, no value, in a temporary tree and copied
     from it. -->
<xsl:stylesheet version="2.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform">
  <xsl:template match="/">
    <out>
      <xsl:processing-instruction name="p1">data</xsl:processing-instruction>
      <xsl:processing-instruction name="p2" select="'  leading spaces'"/>
      <xsl:processing-instruction name="p3" select="'a?>b', 'c'"/>
      <xsl:processing-instruction name="{'p4'}"/>
      <xsl:processing-instruction name="p5"><xsl:value-of select="/r/@b"/>?&gt; &#9;</xsl:processing-instruction>
      <xsl:processing-instruction name=" p6 ">x</xsl:processing-instruction>
      <in-variable><xsl:variable name="v"><xsl:processing-instruction name="pv">x</xsl:processing-instruction></xsl:variable><xsl:copy-of select="$v"/><xsl:value-of select="name($v/node()), string($v/node())"/></in-variable>
    </out>
  </xsl:template>
</xsl:stylesheet>
