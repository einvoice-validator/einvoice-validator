<?xml version="1.0" encoding="UTF-8"?>
<!-- Match patterns: paths, //, anchors, predicates (positional or not), current(),
     kind tests, wildcards, root(), parenthesized patterns. -->
<xsl:stylesheet version="2.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform" xmlns:p="urn:p">
  <xsl:output indent="no"/>
  <xsl:variable name="wanted" select="'2'"/>
  <xsl:template match="/">
    <out>
      <xsl:for-each select="//node() | //@*">
        <m><xsl:apply-templates select="." mode="a"/>|<xsl:apply-templates select="." mode="b"/>|<xsl:apply-templates select="." mode="c"/>|<xsl:apply-templates select="." mode="d"/></m>
      </xsl:for-each>
    </out>
  </xsl:template>
  <xsl:template match="node() | @*" mode="a b c d" priority="-10">-</xsl:template>
  <xsl:template match="x[2]" mode="a">x[2]</xsl:template>
  <xsl:template match="x[last()]" mode="a">x[last()]</xsl:template>
  <xsl:template match="x[position() = 1]" mode="a">x[position()=1]</xsl:template>
  <xsl:template match="amount[. = 2]" mode="a">amount[.=2]</xsl:template>
  <xsl:template match="@*[. = '3']" mode="a">@*[.=3]</xsl:template>
  <xsl:template match="text()[normalize-space()][1]" mode="a">text()[1]</xsl:template>
  <xsl:template match="/r/x[@n = $wanted]" mode="b">/r/x[@n=$wanted]</xsl:template>
  <xsl:template match="r//w" mode="b">r//w</xsl:template>
  <xsl:template match="//amounts/amount[2]" mode="b">//amounts/amount[2]</xsl:template>
  <xsl:template match="/r" mode="b">/r</xsl:template>
  <xsl:template match="y/text()" mode="b">y/text()</xsl:template>
  <xsl:template match="r/@*" mode="b">r/@*</xsl:template>
  <xsl:template match="*[@n = current()/@n + 1]" mode="c">never</xsl:template>
  <xsl:template match="x[following-sibling::x[@n = current()/@n + 1]]" mode="c">x-with-next</xsl:template>
  <xsl:template match="element(x)" mode="c">element(x)</xsl:template>
  <xsl:template match="attribute(n)" mode="c">attribute(n)</xsl:template>
  <xsl:template match="p:*" mode="c">p:*</xsl:template>
  <xsl:template match="@p:*" mode="c">@p:*</xsl:template>
  <xsl:template match="*:x" mode="c">*:x</xsl:template>
  <xsl:template match="comment() | processing-instruction()" mode="c">comment-or-pi</xsl:template>
  <xsl:template match="root()" mode="d">root()</xsl:template>
  <xsl:template match="(/r/x | /r/y)[@n != '1' or @z]" mode="d">(x|y)[p]</xsl:template>
  <xsl:template match="/" mode="d">/</xsl:template>
  <xsl:template match="amounts//text()" mode="d">amounts//text()</xsl:template>
</xsl:stylesheet>
