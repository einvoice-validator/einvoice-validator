<?xml version="1.0" encoding="UTF-8"?>
<!-- A named template calling itself last (a tail call) over each of 10 000 characters, in
     an xsl:choose and from an xsl:if, the result built as it goes. -->
<xsl:stylesheet version="2.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform">
  <xsl:template match="/">
    <out>
      <xsl:variable name="counted"><xsl:call-template name="count"><xsl:with-param name="s" select="string-join(for $i in 1 to 5000 return 'a(', '')"/></xsl:call-template></xsl:variable>
      <count><xsl:value-of select="$counted"/></count>
      <xsl:call-template name="emit"><xsl:with-param name="n" select="5"/></xsl:call-template>
    </out>
  </xsl:template>
  <xsl:template name="count">
    <xsl:param name="s"/>
    <xsl:param name="n" select="0"/>
    <xsl:choose>
      <xsl:when test="$n lt 0 or string-length($s) = 0"><xsl:value-of select="$n"/></xsl:when>
      <xsl:when test="starts-with($s, '(')"><xsl:call-template name="count"><xsl:with-param name="s" select="substring($s, 2)"/><xsl:with-param name="n" select="$n + 1"/></xsl:call-template></xsl:when>
      <xsl:otherwise><xsl:call-template name="count"><xsl:with-param name="s" select="substring($s, 2)"/><xsl:with-param name="n" select="$n"/></xsl:call-template></xsl:otherwise>
    </xsl:choose>
  </xsl:template>
  <xsl:template name="emit">
    <xsl:param name="n"/>
    <e n="{$n}"/>
    <xsl:if test="$n gt 0"><xsl:call-template name="emit"><xsl:with-param name="n" select="$n - 1"/></xsl:call-template></xsl:if>
  </xsl:template>
</xsl:stylesheet>
