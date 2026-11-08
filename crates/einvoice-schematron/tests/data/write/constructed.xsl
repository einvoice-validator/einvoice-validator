<?xml version="1.0" encoding="UTF-8"?>
<!-- A result tree XSLT constructs: text made of several pieces, an empty text node,
     namespaces made with xsl:namespace and copied, a comment with a double hyphen,
     which the serializer of Saxon writes as XML and DOMWriter builds as a DOM. -->
<xsl:stylesheet xmlns:xsl="http://www.w3.org/1999/XSL/Transform" xmlns:k="urn:k" version="2.0">
  <xsl:template match="/">
    <xsl:element name="xsl:stylesheet" namespace="http://www.w3.org/1999/XSL/Transform">
      <xsl:namespace name="made">urn:made</xsl:namespace>
      <xsl:attribute name="version">2.0</xsl:attribute>
      <xsl:value-of select="'a'"/><xsl:value-of select="''"/><xsl:text/><xsl:value-of select="'b'"/>
      <xsl:comment>a -- b -</xsl:comment>
      <k:literal k:attr="1"><xsl:copy-of select="/*/namespace::*"/></k:literal>
      <xsl:element name="made:e" namespace="urn:made"/>
    </xsl:element>
  </xsl:template>
</xsl:stylesheet>
