<?xml version="1.0" encoding="UTF-8"?>
<!-- Error: a function argument that cannot be converted to the parameter type. -->
<xsl:stylesheet version="2.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform" xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:f="urn:f">
  <xsl:function name="f:inc" as="xs:integer"><xsl:param name="n" as="xs:integer"/><xsl:sequence select="$n + 1"/></xsl:function>
  <xsl:template match="/"><out><xsl:value-of select="f:inc(/r/@b)"/></out></xsl:template>
</xsl:stylesheet>
