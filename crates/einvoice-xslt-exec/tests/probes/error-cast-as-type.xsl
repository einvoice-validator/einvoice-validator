<?xml version="1.0" encoding="UTF-8"?>
<!-- Error: `cast as` between types that cannot be cast (xs:date to xs:boolean), at run time. -->
<xsl:stylesheet version="2.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform" xmlns:xs="http://www.w3.org/2001/XMLSchema">
  <xsl:template match="/"><out><xsl:value-of select="(/r/@a, '2013-04-10' cast as xs:date)[2] cast as xs:boolean"/></out></xsl:template>
</xsl:stylesheet>
