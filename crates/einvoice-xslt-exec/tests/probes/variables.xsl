<?xml version="1.0" encoding="UTF-8"?>
<!-- Global and local variables and parameters, temporary trees, `as` conversion,
     xsl:call-template. -->
<xsl:stylesheet version="2.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform" xmlns:xs="http://www.w3.org/2001/XMLSchema">
  <xsl:output indent="no"/>
  <xsl:param name="p-absent"/>
  <xsl:param name="p-default" select="count(//x)"/>
  <xsl:param name="p-typed" as="xs:integer" select="7"/>
  <xsl:variable name="g-later" select="$g-earlier + 1"/>
  <xsl:variable name="g-earlier" select="10"/>
  <xsl:variable name="g-unused-error" select="xs:integer('not a number')"/>
  <xsl:variable name="g-tree"><t a="1">x<u>y</u></t><t a="2"/></xsl:variable>
  <xsl:variable name="g-context" select="name(/*)"/>
  <xsl:variable name="g-typed" as="xs:decimal*"><xsl:sequence select="/r/amounts/amount"/></xsl:variable>
  <xsl:template match="/">
    <out>
      <params>[<xsl:value-of select="$p-absent"/>][<xsl:value-of select="$p-default"/>][<xsl:value-of select="$p-typed"/>]</params>
      <globals><xsl:value-of select="$g-later, $g-context"/></globals>
      <tree><xsl:value-of select="count($g-tree/t), $g-tree/t[2]/@a, string($g-tree), name($g-tree/*[1]/*), count($g-tree/t/node())"/></tree>
      <typed><xsl:value-of select="sum($g-typed), $g-typed[1] instance of xs:decimal"/></typed>
      <local>
        <xsl:variable name="v" select="1"/>
        <xsl:for-each select="/r/x"><xsl:variable name="v" select="concat('in', $v, @n)"/><xsl:value-of select="$v"/>;</xsl:for-each>
        <xsl:value-of select="$v"/>
      </local>
      <local-tree><xsl:variable name="t"><xsl:for-each select="/r/x"><i><xsl:value-of select="@n"/></i></xsl:for-each></xsl:variable><xsl:value-of select="count($t/i), sum($t/i)"/></local-tree>
      <local-unused><xsl:variable name="bad" select="xs:integer('x')"/><xsl:value-of select="'ok'"/></local-unused>
      <local-lazy><xsl:variable name="seq" select="for $s in ('1', 'x') return xs:integer($s)"/><xsl:variable name="ok" select="true()"/><xsl:if test="not($ok)"><xsl:value-of select="$seq"/></xsl:if>lazy</local-lazy>
      <typed-local><xsl:variable name="d" as="xs:decimal" select="/r/amounts/amount[1]"/><xsl:value-of select="$d * 2, $d instance of xs:decimal"/></typed-local>
      <typed-content><xsl:variable name="s" as="xs:string*"><xsl:sequence select="'a', 'b'"/><xsl:sequence select="/r/x/@n"/></xsl:variable><xsl:value-of select="count($s), $s" separator=","/></typed-content>
      <call><xsl:apply-templates select="/r/y" mode="call"/></call>
      <call-defaults><xsl:call-template name="named"/></call-defaults>
      <with-param-tree><xsl:call-template name="named"><xsl:with-param name="a"><b>tree</b></xsl:with-param><xsl:with-param name="n" select="/r/@p:c" xmlns:p="urn:p"/></xsl:call-template></with-param-tree>
      <with-param-seq><xsl:apply-templates select="/r/x[1]" mode="seq"><xsl:with-param name="s" select="/r/x/@n"/></xsl:apply-templates></with-param-seq>
    </out>
  </xsl:template>
  <xsl:template match="y" mode="call"><xsl:variable name="here" select="name()"/><xsl:call-template name="named"><xsl:with-param name="a" select="$here"/></xsl:call-template></xsl:template>
  <xsl:template name="named"><xsl:param name="a" select="'default-a'"/><xsl:param name="n" as="xs:integer" select="1"/>[<xsl:value-of select="name(), $a, $n + 1, $n instance of xs:integer"/>]</xsl:template>
  <xsl:template match="x" mode="seq"><xsl:param name="s"/><xsl:value-of select="count($s), $s"/></xsl:template>
</xsl:stylesheet>
