<?xml version="1.0" encoding="UTF-8"?>
<!-- A result tree with an attribute whose name has a character outside the Basic
     Multilingual Plane (Saxon makes it, the JDK's DOM refuses it). -->
<xsl:stylesheet xmlns:xsl="http://www.w3.org/1999/XSL/Transform" version="2.0">
  <xsl:template match="/">
    <r><xsl:attribute name="{concat('a', codepoints-to-string(65536))}">1</xsl:attribute></r>
  </xsl:template>
</xsl:stylesheet>
