<?xml version="1.0" encoding="UTF-8"?>
<!-- xsl:copy of each kind of item: a document node (its children in complex content), an
     element (its namespaces, the body), an attribute, a text node, a comment and a
     processing instruction (the body not evaluated: no message), atomic values; in a
     temporary tree, recursively, and at the top of a sequence (parentless copies). -->
<xsl:stylesheet version="2.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform" xmlns:p="urn:p" xmlns:f="urn:f">
  <xsl:template match="/">
    <out>
      <doc><xsl:for-each select="/"><xsl:copy><k/></xsl:copy></xsl:for-each></doc>
      <elem><xsl:for-each select="/r/p:x"><xsl:copy><xsl:attribute name="added">1</xsl:attribute><xsl:value-of select="."/></xsl:copy></xsl:for-each></elem>
      <attr><xsl:for-each select="/r/@*"><xsl:copy/></xsl:for-each></attr>
      <text><xsl:for-each select="/r/x[1]/text()"><xsl:copy><xsl:message>not evaluated</xsl:message></xsl:copy></xsl:for-each></text>
      <comment><xsl:for-each select="/r/comment()"><xsl:copy><xsl:message>not evaluated</xsl:message></xsl:copy></xsl:for-each></comment>
      <pi><xsl:for-each select="/r/processing-instruction()"><xsl:copy/></xsl:for-each></pi>
      <atomic><xsl:for-each select="(1, 'a', 2.5)"><xsl:copy/></xsl:for-each></atomic>
      <in-variable>
        <xsl:variable name="v"><xsl:for-each select="/r/y"><xsl:copy><xsl:copy-of select="@*"/><xsl:apply-templates mode="copy"/></xsl:copy></xsl:for-each></xsl:variable>
        <xsl:value-of select="count($v/y/node())"/>
        <xsl:copy-of select="$v"/>
      </in-variable>
      <in-function><xsl:variable name="s" select="f:copies(/r)"/><xsl:value-of select="count($s), for $n in $s return name($n), count($s[1]/..)" separator=","/></in-function>
    </out>
  </xsl:template>
  <xsl:template match="node()" mode="copy"><xsl:copy><xsl:apply-templates mode="copy"/></xsl:copy></xsl:template>
  <xsl:function name="f:copies">
    <xsl:param name="r"/>
    <xsl:for-each select="$r/x[1], $r/@a, $r/comment(), $r/processing-instruction()"><xsl:copy/></xsl:for-each>
  </xsl:function>
</xsl:stylesheet>
