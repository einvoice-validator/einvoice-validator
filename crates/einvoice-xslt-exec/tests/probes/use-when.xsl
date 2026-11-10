<?xml version="1.0" encoding="UTF-8"?>
<!-- use-when, statically: on xsl:when (Saxon has xsl:namespace), on literal result
     elements (xsl:use-when, the system properties), on declarations; what it excludes is
     not read at all (an unknown instruction, an expression that does not parse, a
     duplicate template). -->
<xsl:stylesheet version="2.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform" xmlns:f="urn:f">
  <xsl:template match="/">
    <out>
      <xsl:choose>
        <xsl:when test="false()"/>
        <xsl:when use-when="not(element-available('xsl:namespace'))" test="true()"><excluded/></xsl:when>
        <xsl:when use-when="element-available('xsl:namespace')" test="true()"><kept/></xsl:when>
      </xsl:choose>
      <lre xsl:use-when="system-property('xsl:vendor') = 'Saxonica'"/>
      <gone xsl:use-when="system-property('xsl:vendor') != 'Saxonica'"><xsl:no-such-instruction/></gone>
      <xsl:if use-when="false()" test="this is ((( not XPath"/>
      <xsl:if use-when="element-available('xsl:import-schema') or true()" test="true()"><or/></xsl:if>
      <v><xsl:value-of select="f:x()"/></v>
    </out>
  </xsl:template>
  <xsl:template match="/" use-when="false()"><never/></xsl:template>
  <xsl:function name="f:x" use-when="element-available('xsl:import-schema')"><xsl:sequence select="'excluded'"/></xsl:function>
  <xsl:function name="f:x" use-when="not(element-available('xsl:import-schema'))"><xsl:sequence select="'kept'"/></xsl:function>
</xsl:stylesheet>
