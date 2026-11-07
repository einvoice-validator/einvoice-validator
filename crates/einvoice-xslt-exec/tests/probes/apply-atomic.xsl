<?xml version="1.0" encoding="UTF-8"?>
<!-- xsl:apply-templates on atomic values: Saxon (an XSLT 3.0 processor) writes them as text. -->
<xsl:stylesheet version="2.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform" xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:f="urn:f">
  <xsl:template match="/"><out><xsl:apply-templates select="/r/x/string(@n)"/></out></xsl:template>
</xsl:stylesheet>
