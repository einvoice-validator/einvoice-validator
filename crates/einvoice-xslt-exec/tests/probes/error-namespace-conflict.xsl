<?xml version="1.0" encoding="UTF-8"?>
<!-- xsl:namespace binding a prefix the element binds to another URI: XTDE0430. -->
<xsl:stylesheet version="2.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform">
  <xsl:template match="/"><out><a xmlns:c="urn:1"><xsl:namespace name="c" select="'urn:2'"/></a></out></xsl:template>
</xsl:stylesheet>
