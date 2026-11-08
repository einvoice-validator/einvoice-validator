<?xml version="1.0" encoding="UTF-8"?>
<!-- Error: `cast as` of an exponent, not an xs:decimal. -->
<xsl:stylesheet version="2.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform" xmlns:xs="http://www.w3.org/2001/XMLSchema">
  <xsl:template match="/"><out><xsl:value-of select="'1e3' cast as xs:decimal"/></out></xsl:template>
</xsl:stylesheet>
