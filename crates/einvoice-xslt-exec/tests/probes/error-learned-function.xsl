<?xml version="1.0" encoding="UTF-8"?>
<!-- Error: in a function body, a local variable evaluated 20 times, read or not, is
     evaluated where it is bound from the 21st evaluation on (Saxon's learning). -->
<xsl:stylesheet version="2.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform" xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:f="urn:f">
  <xsl:function name="f:v" as="xs:boolean">
    <xsl:param name="s" as="xs:string"/>
    <xsl:variable name="m" select="xs:decimal(substring($s, 2))"/>
    <xsl:sequence select="starts-with($s, 'y') and ($m = 1 or $m = 2)"/>
  </xsl:function>
  <xsl:template match="/"><out><xsl:value-of select="for $i in (1 to 20) return f:v(concat('n', /r/@a)), f:v(concat('n', /r/@b))"/></out></xsl:template>
</xsl:stylesheet>
