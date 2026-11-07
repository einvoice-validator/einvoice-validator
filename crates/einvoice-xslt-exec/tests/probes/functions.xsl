<?xml version="1.0" encoding="UTF-8"?>
<!-- xsl:function: conversion of arguments and result, recursion, global variables,
     control flow in the body. -->
<xsl:stylesheet version="2.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform" xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:f="urn:f">
  <xsl:output indent="no"/>
  <xsl:variable name="factor" select="3"/>
  <xsl:function name="f:add" as="xs:decimal">
    <xsl:param name="a" as="xs:decimal"/>
    <xsl:param name="b" as="xs:decimal"/>
    <xsl:sequence select="$a + $b"/>
  </xsl:function>
  <xsl:function name="f:fact" as="xs:integer">
    <xsl:param name="n" as="xs:integer"/>
    <xsl:choose>
      <xsl:when test="$n le 1"><xsl:sequence select="1"/></xsl:when>
      <xsl:otherwise>
        <xsl:variable name="rest" select="f:fact($n - 1)"/>
        <xsl:sequence select="$n * $rest"/>
      </xsl:otherwise>
    </xsl:choose>
  </xsl:function>
  <xsl:function name="f:scale">
    <xsl:param name="v"/>
    <xsl:sequence select="for $i in $v return $i * $factor"/>
  </xsl:function>
  <xsl:function name="f:untyped">
    <xsl:param name="v"/>
    <xsl:sequence select="$v"/>
  </xsl:function>
  <xsl:function name="f:allowed" as="xs:boolean">
    <xsl:param name="s" as="xs:string?"/>
    <xsl:variable name="chars">ABC</xsl:variable>
    <xsl:sequence select="string-length(translate($s, $chars, '')) = 0"/>
  </xsl:function>
  <xsl:function name="f:arity">
    <xsl:param name="a"/>
    <xsl:sequence select="'one'"/>
  </xsl:function>
  <xsl:function name="f:arity">
    <xsl:param name="a"/>
    <xsl:param name="b"/>
    <xsl:sequence select="'two'"/>
  </xsl:function>
  <xsl:function name="f:nodes" as="element()*">
    <xsl:param name="r" as="node()"/>
    <xsl:sequence select="$r//x[@n]"/>
  </xsl:function>
  <xsl:template match="/">
    <out>
      <add><xsl:value-of select="f:add(/r/amounts/amount[1], 2), f:add(1, 2) instance of xs:decimal"/></add>
      <fact><xsl:value-of select="f:fact(20), f:fact(xs:integer(/r/@a))"/></fact>
      <scale><xsl:value-of select="f:scale((1, 2.5))"/></scale>
      <untyped><xsl:value-of select="f:untyped(/r/@a) instance of attribute(), f:untyped(/r/@a) = 1"/></untyped>
      <allowed><xsl:value-of select="f:allowed('CAB'), f:allowed('ABD'), f:allowed(())"/></allowed>
      <arity><xsl:value-of select="f:arity(1), f:arity(1, 2)"/></arity>
      <nodes><xsl:value-of select="count(f:nodes(/)), f:nodes(/)/@n"/></nodes>
    </out>
  </xsl:template>
</xsl:stylesheet>
