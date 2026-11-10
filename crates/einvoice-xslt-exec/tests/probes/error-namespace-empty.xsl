<?xml version="1.0" encoding="UTF-8"?>
<!-- xsl:namespace with an empty URI. -->
<xsl:stylesheet version="2.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform">
  <xsl:template match="/"><out><a><xsl:namespace name="x" select="replace(name(/r), 'r', '')"/></a></out></xsl:template>
</xsl:stylesheet>
