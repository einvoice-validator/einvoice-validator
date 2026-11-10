<?xml version="1.0" encoding="UTF-8"?>
<!-- Imported by namespace-alias.xsl: declares the alias, and uses it too. -->
<xsl:stylesheet version="2.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform"
    xmlns:axsl="urn:alias" xmlns:m="urn:module">
  <xsl:namespace-alias stylesheet-prefix="axsl" result-prefix="xsl"/>
  <xsl:template name="from-the-module"><axsl:value-of select="."/><m:e/></xsl:template>
</xsl:stylesheet>
