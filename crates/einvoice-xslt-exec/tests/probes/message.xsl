<?xml version="1.0" encoding="UTF-8"?>
<!-- xsl:message: text, elements and escapes written as XML, line ends, an empty message
     (nothing printed), atomic values, comments and processing instructions, terminate no
     and computed, in order with the result. -->
<xsl:stylesheet version="2.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform">
  <xsl:template match="/">
    <out>
      <xsl:message>plain <b a="1">bold</b> &amp; after</xsl:message>
      <xsl:message terminate="no">second
line</xsl:message>
      <xsl:message/>
      <xsl:message terminate="{'no'}"><xsl:sequence select="1, 2"/><xsl:comment>c</xsl:comment><xsl:processing-instruction name="p">d</xsl:processing-instruction></xsl:message>
      <xsl:message><xsl:value-of select="/r/@b"/></xsl:message>
      <xsl:message><x/></xsl:message>
      <xsl:message>   </xsl:message>
      <xsl:message terminate="false">C1: &#x85; &lt;</xsl:message>
      <done/>
    </out>
  </xsl:template>
</xsl:stylesheet>
