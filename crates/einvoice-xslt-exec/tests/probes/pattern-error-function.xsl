<?xml version="1.0" encoding="UTF-8"?>
<!-- A stylesheet function called from a pattern fails (its argument or its result does not have the declared type): the rule does not match, with a warning. -->
<xsl:stylesheet version="2.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform" xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:f="urn:f">
  <xsl:output indent="no"/>
  <xsl:function name="f:one" as="xs:string">
    <xsl:param name="s" as="xs:string"/>
    <xsl:sequence select="$s"/>
  </xsl:function>
  <xsl:function name="f:many" as="xs:string">
    <xsl:param name="n"/>
    <xsl:sequence select="$n/*/name()"/>
  </xsl:function>
  <xsl:template match="/"><out><xsl:apply-templates select="/r/*"/></out></xsl:template>
  <xsl:template match="*[f:one(text()) = 't2']" priority="3">one</xsl:template>
  <xsl:template match="amounts[f:many(.) = 'amount']" priority="3">many</xsl:template>
  <xsl:template match="*">[<xsl:value-of select="name()"/>]</xsl:template>
</xsl:stylesheet>
