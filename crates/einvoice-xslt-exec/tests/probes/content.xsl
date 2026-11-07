<?xml version="1.0" encoding="UTF-8"?>
<!-- Constructing complex and simple content (XSLT 2.0 section 5.7). -->
<xsl:stylesheet version="2.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform" xmlns:p="urn:p">
  <xsl:output indent="no"/>
  <xsl:template match="/">
    <out>
      <atomics><xsl:sequence select="1, 2"/><xsl:text>a</xsl:text><xsl:sequence select="3"/><xsl:value-of select="''"/><xsl:sequence select="4"/><xsl:sequence select="'s', 1.50, true()"/></atomics>
      <attr-content><xsl:attribute name="v"><xsl:sequence select="1, 2"/><xsl:text>a</xsl:text><xsl:sequence select="3"/><xsl:value-of select="''"/><xsl:sequence select="4"/></xsl:attribute></attr-content>
      <attr-select><xsl:attribute name="v" select="1, 'b', /r/x/@n"/></attr-select>
      <attr-nodes><xsl:attribute name="v"><xsl:sequence select="/r/x, /r/@a"/></xsl:attribute></attr-nodes>
      <attr-replace a="literal"><xsl:attribute name="a">computed</xsl:attribute><xsl:attribute name="a">last</xsl:attribute></attr-replace>
      <value-of-select><xsl:value-of select="/r/x"/>|<xsl:value-of select="/r/y/text()"/>|<xsl:value-of select="/r/x/@n" separator=", "/>|<xsl:value-of select="()"/>|<xsl:value-of select="/r/amounts/amount/number(.)" separator="{'+'}"/></value-of-select>
      <value-of-content><xsl:value-of><xsl:sequence select="1, 2"/><xsl:text>t</xsl:text><xsl:value-of select="'u'"/></xsl:value-of>|<xsl:value-of separator="-"><xsl:sequence select="1, 2"/><xsl:text>t</xsl:text><xsl:text>v</xsl:text></xsl:value-of></value-of-content>
      <copy><xsl:sequence select="/r/@b"/><xsl:sequence select="/r/y"/></copy>
      <copy-attr-first><xsl:sequence select="/r/@a, /r/@p:c"/>text</copy-attr-first>
      <copy-document><xsl:variable name="tree"><a b="c">d<e/></a></xsl:variable><xsl:sequence select="$tree"/></copy-document>
      <comments><xsl:comment>a--b-</xsl:comment><xsl:comment>-x</xsl:comment><xsl:comment><xsl:value-of select="/r/text"/></xsl:comment><xsl:comment/></comments>
      <text-escapes t="{/r/text}"><xsl:value-of select="/r/text"/></text-escapes>
      <empty-element/>
      <empty-with-attribute><xsl:attribute name="a" select="''"/></empty-with-attribute>
      <avt a="{/r/x/@n}" b="{{literal}}" c="x{1 + 1}y{()}z" d="{/r/amounts/amount}"/>
      <numbers><xsl:value-of select="1.0, 1.5e0, 0.1 + 0.2, 1e10, xs:decimal('1.50'), 10 div 4, 10 idiv 4, -0.0e0" separator=" " xmlns:xs="http://www.w3.org/2001/XMLSchema"/></numbers>
    </out>
  </xsl:template>
</xsl:stylesheet>
