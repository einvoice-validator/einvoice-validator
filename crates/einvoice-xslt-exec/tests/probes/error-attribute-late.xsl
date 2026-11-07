<?xml version="1.0" encoding="UTF-8"?>
<!-- Error: an attribute created after the children of its element. -->
<xsl:stylesheet version="2.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform" xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:f="urn:f">
  <xsl:template match="/"><out><a/><xsl:attribute name="late" select="1"/></out></xsl:template>
</xsl:stylesheet>
