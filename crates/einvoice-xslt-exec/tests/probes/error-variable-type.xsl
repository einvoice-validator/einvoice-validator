<?xml version="1.0" encoding="UTF-8"?>
<!-- Error: a local variable whose value does not match its type. -->
<xsl:stylesheet version="2.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform" xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:f="urn:f">
  <xsl:template match="/"><out><xsl:variable name="v" as="xs:integer" select="/r/x/@n"/><xsl:value-of select="$v"/></out></xsl:template>
</xsl:stylesheet>
