<?xml version="1.0" encoding="UTF-8"?>
<!-- A result tree with a processing instruction whose target has U+0132 (Saxon makes
     it, the JDK's DOM refuses it). -->
<xsl:stylesheet xmlns:xsl="http://www.w3.org/1999/XSL/Transform" version="2.0">
  <xsl:template match="/">
    <r><xsl:processing-instruction name="{concat('t', codepoints-to-string(306))}">d</xsl:processing-instruction></r>
  </xsl:template>
</xsl:stylesheet>
