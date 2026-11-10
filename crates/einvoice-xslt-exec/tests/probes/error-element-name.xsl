<?xml version="1.0" encoding="UTF-8"?>
<!-- xsl:element with a computed name that is not a QName. -->
<xsl:stylesheet version="2.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform">
  <xsl:template match="/"><out><xsl:element name="{replace(name(/r), 'r', '1bad')}"/></out></xsl:template>
</xsl:stylesheet>
