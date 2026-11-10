<?xml version="1.0" encoding="UTF-8"?>
<!-- xsl:copy in a function, which has no context item. -->
<xsl:stylesheet version="2.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform" xmlns:f="urn:f">
  <xsl:function name="f:c"><xsl:copy/></xsl:function>
  <xsl:template match="/"><out><xsl:copy-of select="f:c()"/></out></xsl:template>
</xsl:stylesheet>
