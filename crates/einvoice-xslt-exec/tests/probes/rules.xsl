<?xml version="1.0" encoding="UTF-8"?>
<!-- Template rule selection: priorities, ties, union patterns, next-match, modes and the
     built-in rules. -->
<xsl:stylesheet version="2.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform" xmlns:p="urn:p">
  <xsl:output indent="no"/>
  <xsl:template match="/">
    <out>
      <tie><xsl:apply-templates select="/r/x[1]" mode="tie"/></tie>
      <explicit><xsl:apply-templates select="/r/x[1]" mode="explicit"/></explicit>
      <split><xsl:apply-templates select="/r/x[1]" mode="split"/></split>
      <split-equal><xsl:apply-templates select="/r/x[1]" mode="split-equal"/></split-equal>
      <default-priorities><xsl:apply-templates select="/r/node() | /r/@*" mode="dp"/></default-priorities>
      <all><xsl:apply-templates select="/r/y" mode="m1"/>|<xsl:apply-templates select="/r/y" mode="m2"/></all>
      <current><xsl:apply-templates select="/r" mode="cur"/></current>
      <builtin-default><xsl:apply-templates select="/r/y"/></builtin-default>
      <builtin-named><xsl:apply-templates select="/r/@a | /r/y | /r/comment() | /r/processing-instruction() | /" mode="none"/></builtin-named>
      <builtin-params><xsl:apply-templates select="/r/y" mode="bp"><xsl:with-param name="v" select="'passed'"/></xsl:apply-templates></builtin-params>
      <next-param><xsl:apply-templates select="/r/x[2]" mode="np"/></next-param>
      <positions><xsl:apply-templates select="/r/x" mode="pos"/></positions>
      <no-select><xsl:apply-templates select="/r/y" mode="children"/></no-select>
    </out>
  </xsl:template>

  <xsl:template match="x" mode="tie">first</xsl:template>
  <xsl:template match="x" mode="tie">second</xsl:template>

  <xsl:template match="x | *" mode="explicit" priority="1">[A<xsl:next-match/>]</xsl:template>
  <xsl:template match="x" mode="explicit" priority="0">[B<xsl:next-match/>]</xsl:template>

  <xsl:template match="x | *" mode="split">[S<xsl:next-match/>]</xsl:template>

  <xsl:template match="x | element(x)" mode="split-equal">[E<xsl:next-match/>]</xsl:template>
  <xsl:template match="r/x" mode="split-equal">[F<xsl:next-match/>]</xsl:template>

  <xsl:template match="node()" mode="dp">(node)</xsl:template>
  <xsl:template match="*" mode="dp">(*)</xsl:template>
  <xsl:template match="p:*" mode="dp">(p:*)</xsl:template>
  <xsl:template match="*:x" mode="dp">(*:x)</xsl:template>
  <xsl:template match="x" mode="dp">(x)</xsl:template>
  <xsl:template match="x[@n = '2']" mode="dp">(x[2])</xsl:template>
  <xsl:template match="text()" mode="dp">(text)</xsl:template>
  <xsl:template match="@*" mode="dp">(@*)</xsl:template>
  <xsl:template match="@p:*" mode="dp">(@p:*)</xsl:template>
  <xsl:template match="@a" mode="dp">(@a)</xsl:template>
  <xsl:template match="comment()" mode="dp">(comment)</xsl:template>

  <xsl:template match="y" mode="#all">[all <xsl:apply-templates select="w" mode="#current"/>]</xsl:template>
  <xsl:template match="w" mode="m1">w1</xsl:template>
  <xsl:template match="w" mode="m2 #default">w2</xsl:template>

  <xsl:template match="r" mode="cur"><xsl:for-each select="x"><xsl:apply-templates select="." mode="#current"/></xsl:for-each></xsl:template>
  <xsl:template match="x" mode="cur">C<xsl:value-of select="@n"/></xsl:template>

  <xsl:template match="w" mode="bp"><xsl:param name="v" select="'default'"/>[<xsl:value-of select="$v"/>]</xsl:template>

  <xsl:template match="x" mode="np" priority="2"><xsl:param name="v" select="'d2'"/>{<xsl:value-of select="$v"/><xsl:next-match><xsl:with-param name="v" select="'from-next-match'"/></xsl:next-match>}</xsl:template>
  <xsl:template match="x" mode="np" priority="1"><xsl:param name="v" select="'d1'"/>{<xsl:value-of select="$v"/><xsl:next-match/>}</xsl:template>
  <xsl:template match="x" mode="np"><xsl:param name="v" select="'d0'"/>{<xsl:value-of select="$v"/>}</xsl:template>

  <xsl:template match="x" mode="pos"><xsl:value-of select="position(), last()" separator="/"/>;</xsl:template>

  <xsl:template match="y" mode="children">[<xsl:apply-templates mode="children"/>]</xsl:template>
  <xsl:template match="text()" mode="children">(<xsl:value-of select="."/>)</xsl:template>
</xsl:stylesheet>
