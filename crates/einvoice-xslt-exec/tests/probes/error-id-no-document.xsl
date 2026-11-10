<?xml version="1.0" encoding="UTF-8"?>
<!-- id() on a tree whose root is not a document node. -->
<xsl:stylesheet version="2.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform">
  <xsl:template match="/">
    <xsl:variable name="e" as="element()"><e xml:id="q"/></xsl:variable>
    <out><xsl:value-of select="count(id('q', $e))"/></out>
  </xsl:template>
</xsl:stylesheet>
