<?xml version="1.0" encoding="UTF-8"?>
<!-- Error: `cast as` of a string that is not a boolean. -->
<xsl:stylesheet version="2.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform" xmlns:xs="http://www.w3.org/2001/XMLSchema">
  <xsl:template match="/"><out><xsl:value-of select="/r/@b cast as xs:boolean"/></out></xsl:template>
</xsl:stylesheet>
