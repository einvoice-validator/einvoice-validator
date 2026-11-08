<?xml version="1.0" encoding="UTF-8"?>
<!-- Error: `cast as` of a sequence of three attributes. -->
<xsl:stylesheet version="2.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform" xmlns:xs="http://www.w3.org/2001/XMLSchema">
  <xsl:template match="/"><out><xsl:value-of select="/r/x/@n cast as xs:integer"/></out></xsl:template>
</xsl:stylesheet>
