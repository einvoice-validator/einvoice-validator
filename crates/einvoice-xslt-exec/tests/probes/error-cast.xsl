<?xml version="1.0" encoding="UTF-8"?>
<!-- Error: a dynamic error in an expression, reported with its code. -->
<xsl:stylesheet version="2.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform" xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:f="urn:f">
  <xsl:template match="/"><out><xsl:value-of select="xs:decimal(/r/@b)"/></out></xsl:template>
</xsl:stylesheet>
