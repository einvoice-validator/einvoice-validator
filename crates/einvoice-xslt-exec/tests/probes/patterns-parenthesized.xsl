<?xml version="1.0" encoding="UTF-8"?>
<!-- XSLT 3.0 parenthesized patterns: without a predicate, `(A | B)` alone and as a step
     (`(A | B)/c`, `x/(A | B)`, `/(A | B)`, `//(A | B)`), with predicates and current(),
     their default priorities (each mode has a rival rule declared before, so that the
     priority decides or a tie is reported) and their explicit priorities. -->
<xsl:stylesheet version="2.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform" xmlns:p="urn:p">
  <xsl:output indent="no"/>
  <xsl:template match="/">
    <out>
      <xsl:for-each select="//node() | //@*">
        <m><xsl:apply-templates select="." mode="a"/>|<xsl:apply-templates select="." mode="b"/>|<xsl:apply-templates select="." mode="c"/>|<xsl:apply-templates select="." mode="d"/>|<xsl:apply-templates select="." mode="e"/>|<xsl:apply-templates select="." mode="f"/>|<xsl:apply-templates select="." mode="g"/></m>
      </xsl:for-each>
    </out>
  </xsl:template>
  <xsl:template match="node() | @*" mode="a b c d e f g" priority="-10">-</xsl:template>
  <!-- (A | B) alone: x[@n] (0.5) and y (0) come first. -->
  <xsl:template match="x[@n]" mode="a">x[@n]</xsl:template>
  <xsl:template match="y" mode="a">y</xsl:template>
  <xsl:template match="(x | y)" mode="a">(x|y)</xsl:template>
  <!-- (A) alone, and a parenthesized wildcard. -->
  <xsl:template match="amount[. = 2]" mode="b">amount[.=2]</xsl:template>
  <xsl:template match="(amount)" mode="b">(amount)</xsl:template>
  <xsl:template match="w" mode="b">w</xsl:template>
  <xsl:template match="(*)" mode="b">(*)</xsl:template>
  <!-- Steps after a parenthesized pattern. -->
  <xsl:template match="x/text()" mode="c">x/text()</xsl:template>
  <xsl:template match="(x | y)/text()" mode="c">(x|y)/text()</xsl:template>
  <xsl:template match="(r/x | r/p:x)/@n" mode="c">(r/x|r/p:x)/@n</xsl:template>
  <xsl:template match="(/r)//w" mode="c">(/r)//w</xsl:template>
  <xsl:template match="(amounts)[amount = 2]/amount[last()]" mode="c">(amounts)[amount=2]/amount[last()]</xsl:template>
  <!-- A parenthesized step after others. -->
  <xsl:template match="r/(x | y)" mode="d">r/(x|y)</xsl:template>
  <xsl:template match="/(r | p:x)" mode="d">/(r|p:x)</xsl:template>
  <xsl:template match="//(amount | codes)[. != '2']" mode="d">//(amount|codes)[.!=2]</xsl:template>
  <xsl:template match="(r)[@a = '1']/(x | y)/text()" mode="d">(r)[@a]/(x|y)/text()</xsl:template>
  <xsl:template match="(x | y)[@n = current()/@n or @z]/@*" mode="d">(x|y)[current()]/@*</xsl:template>
  <xsl:template match="r/(x/text() | amounts/amount)" mode="d">r/(x/text()|amounts/amount)</xsl:template>
  <!-- Explicit priorities: one rule for the whole pattern. -->
  <xsl:template match="(x | y)" priority="1" mode="e">(x|y)@1</xsl:template>
  <xsl:template match="x | y" priority="1" mode="e">x|y@1</xsl:template>
  <xsl:template match="(x | p:x)/text()" priority="2" mode="e">(x|p:x)/text()@2</xsl:template>
  <!-- Two alternatives, one parenthesized. -->
  <xsl:template match="w | (x | amount)[. != '']" mode="f">w|(x|amount)[p]</xsl:template>
  <xsl:template match="(text)" mode="f">(text)</xsl:template>
  <xsl:template match="text | codes" mode="f">text|codes</xsl:template>
  <!-- Nested parentheses. -->
  <xsl:template match="((x | y) | (amount))" mode="g">((x|y)|(amount))</xsl:template>
  <xsl:template match="(r/x | /r/y)//text()" mode="g">(r/x|/r/y)//text()</xsl:template>
  <xsl:template match="x/(@n | text())" mode="g">x/(@n|text())</xsl:template>
</xsl:stylesheet>
