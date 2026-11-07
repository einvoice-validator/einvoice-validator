<?xml version="1.0" encoding="UTF-8"?>
<!-- xsl:number: a count pattern that fails on a node does not count it; the error is a warning. -->
<xsl:stylesheet version="2.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform" xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:f="urn:f">
  <xsl:output indent="no"/>
  <xsl:template match="/">
    <out>
      <xsl:for-each select="/r/*">
        <n>
          <xsl:number count="*[xs:integer(string(@n)) gt 1]"/>
        </n>
      </xsl:for-each>
      <xsl:for-each select="/r/x">
        <m>
          <xsl:number level="multiple" count="*[xs:integer(string(@n)) ge 1]"/>
        </m>
      </xsl:for-each>
    </out>
  </xsl:template>
</xsl:stylesheet>
