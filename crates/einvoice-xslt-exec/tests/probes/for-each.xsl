<?xml version="1.0" encoding="UTF-8"?>
<!-- xsl:for-each over nodes and atomic values, position and size, current(),
     xsl:if and xsl:choose. -->
<xsl:stylesheet version="2.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform">
  <xsl:output indent="no"/>
  <xsl:template match="/">
    <out>
      <atomics><xsl:for-each select="(3, 'b', 1.5)"><xsl:value-of select="., position(), last()" separator=":"/>;</xsl:for-each></atomics>
      <nested><xsl:for-each select="/r/x"><xsl:for-each select="../x[@n &lt; current()/@n]"><xsl:value-of select="@n"/></xsl:for-each>;</xsl:for-each></nested>
      <attributes><xsl:for-each select="/r/@*"><xsl:value-of select="name()"/>=<xsl:value-of select="."/>;</xsl:for-each></attributes>
      <choose><xsl:for-each select="/r/x"><xsl:choose><xsl:when test="@n = '1'">one</xsl:when><xsl:when test="@n = '2'">two</xsl:when><xsl:otherwise>other</xsl:otherwise></xsl:choose><xsl:if test="position() != last()">,</xsl:if></xsl:for-each></choose>
      <empty><xsl:for-each select="()">never</xsl:for-each></empty>
      <document><xsl:value-of select="count(document(document-uri(/)) | /), count(document(document-uri(/))//x), count(document(document-uri(/)) | document(document-uri(/)))"/></document>
    </out>
  </xsl:template>
</xsl:stylesheet>
