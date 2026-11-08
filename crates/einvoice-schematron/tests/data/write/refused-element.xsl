<?xml version="1.0" encoding="UTF-8"?>
<!-- A result tree with an element named U+0132, a name character since XML 1.0 fifth
     edition (Saxon makes it), not in the tables of the JDK's DOM (DOMWriter fails). -->
<xsl:stylesheet xmlns:xsl="http://www.w3.org/1999/XSL/Transform" version="2.0">
  <xsl:template match="/">
    <xsl:element name="r"><xsl:element name="{codepoints-to-string(306)}"/></xsl:element>
  </xsl:template>
</xsl:stylesheet>
