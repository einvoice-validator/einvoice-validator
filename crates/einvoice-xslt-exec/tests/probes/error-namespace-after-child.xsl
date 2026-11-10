<?xml version="1.0" encoding="UTF-8"?>
<!-- xsl:namespace after a child of the element. -->
<xsl:stylesheet version="2.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform">
  <xsl:template match="/"><out><a><b/><xsl:namespace name="x" select="'urn:x'"/></a></out></xsl:template>
</xsl:stylesheet>
