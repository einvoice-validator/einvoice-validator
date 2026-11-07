<?xml version="1.0" encoding="UTF-8"?>
<!-- Error: a function result that does not match the declared type. -->
<xsl:stylesheet version="2.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform" xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:f="urn:f">
  <xsl:function name="f:pair" as="xs:integer"><xsl:param name="n"/><xsl:sequence select="$n, $n"/></xsl:function>
  <xsl:template match="/"><out><xsl:value-of select="f:pair(count(/r/x))"/></out></xsl:template>
</xsl:stylesheet>
