<?xml version="1.0" encoding="UTF-8"?>
<!-- Ties between rules of the same priority: parts of one template, templates in
     #all mode against templates of the mode, rules in different rule chains. -->
<xsl:stylesheet version="2.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform">
  <xsl:output indent="no"/>
  <xsl:template match="/">
    <out>
      <three><xsl:apply-templates select="/r/x[1]" mode="three"/></three>
      <three-after><xsl:apply-templates select="/r/x[1]" mode="three-after"/></three-after>
      <chains><xsl:apply-templates select="/r/x[1]" mode="chains"/></chains>
      <chains-after><xsl:apply-templates select="/r/x[1]" mode="chains-after"/></chains-after>
      <all-first><xsl:apply-templates select="/r/x[1]" mode="m-early"/></all-first>
      <all-last><xsl:apply-templates select="/r/x[1]" mode="m-late"/></all-last>
      <explicit-three><xsl:apply-templates select="/r/x[1]" mode="explicit-three"/></explicit-three>
      <negative><xsl:apply-templates select="/r/x[1]" mode="negative"/></negative>
    </out>
  </xsl:template>

  <xsl:template match="x | element(x) | *:x[@n]" mode="three">[T<xsl:next-match/>]</xsl:template>

  <xsl:template match="x | element(x) | x[@n]" mode="three-after">[A<xsl:next-match/>]</xsl:template>
  <xsl:template match="r/x[1]" mode="three-after" priority="5">[B<xsl:next-match/>]</xsl:template>

  <xsl:template match="*[@n] | node()[1] | x[@n]" mode="chains">[C<xsl:next-match/>]</xsl:template>
  <xsl:template match="*[@n] | node()[. = 't1']" mode="chains-after">[D<xsl:next-match/>]</xsl:template>
  <xsl:template match="r/x" mode="chains-after" priority="1">[E<xsl:next-match/>]</xsl:template>

  <xsl:template match="x" mode="m-early">[early-mode<xsl:next-match/>]</xsl:template>
  <xsl:template match="x" mode="#all">[all<xsl:next-match/>]</xsl:template>
  <xsl:template match="x" mode="m-late">[late-mode<xsl:next-match/>]</xsl:template>

  <xsl:template match="x | x[1] | *" mode="explicit-three" priority="0">[X<xsl:next-match/>]</xsl:template>
  <xsl:template match="x" mode="explicit-three" priority="-1">[Y<xsl:next-match/>]</xsl:template>

  <xsl:template match="x" mode="negative" priority="-0.25">[N1<xsl:next-match/>]</xsl:template>
  <xsl:template match="*:x" mode="negative">[N2<xsl:next-match/>]</xsl:template>
  <xsl:template match="node()" mode="negative" priority="-0.5">[N3<xsl:next-match/>]</xsl:template>
  <xsl:template match="*" mode="negative">[N4<xsl:next-match/>]</xsl:template>
</xsl:stylesheet>
