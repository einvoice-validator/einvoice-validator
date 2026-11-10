<?xml version="1.0" encoding="UTF-8"?>
<!-- xsl:namespace-alias declared in an imported module (modules/alias.xsl) and used here:
     element and attribute names in the literal namespace written in the target namespace
     with the result prefix, the literal namespace not copied, the target namespace copied,
     the extension element prefix not copied. -->
<xsl:stylesheet version="2.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform"
    xmlns:axsl="urn:alias" xmlns:o="urn:other" xmlns:exsl="http://exslt.org/common"
    extension-element-prefixes="exsl">
  <xsl:import href="modules/alias.xsl"/>
  <xsl:template match="/">
    <axsl:stylesheet version="2.0" o:attr="1" axsl:attr="2">
      <axsl:template match="/"><lit><xsl:call-template name="from-the-module"/></lit></axsl:template>
    </axsl:stylesheet>
  </xsl:template>
</xsl:stylesheet>
