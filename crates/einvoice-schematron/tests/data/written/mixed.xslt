<?xml version="1.0" encoding="UTF-8" standalone="no"?>
<!--before the root-->
<?before data?>
<xsl:stylesheet xmlns:svrl="http://purl.oclc.org/dsdl/svrl" xmlns:xsl="http://www.w3.org/1999/XSL/Transform" version="2.0">
  <!--first child-->
  <xsl:template match="/">text first<xsl:text>t</xsl:text>text between<!--comment after text--><?pi after a comment?>
    <xsl:if test="1" />  <xsl:if test="2" />
<xsl:if test="3">only text</xsl:if>
    <xsl:if test="4">
      <!--only a comment-->
    </xsl:if>
    <xsl:if test="5">
<?only-pi?>
    </xsl:if>
    <xsl:if test="6">   </xsl:if>text last</xsl:template>
  <xsl:template name="t">
    <xsl:if test="1">
      <xsl:if test="2">
        <xsl:if test="3" />tail      </xsl:if>
    </xsl:if>
    
<?empty?>
  </xsl:template>
  <!--last child-->
</xsl:stylesheet>
<!--after the root-->
<?after?>
