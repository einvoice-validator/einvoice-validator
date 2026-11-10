<?xml version="1.0" encoding="UTF-8"?>
<!-- xsl:namespace binding the prefix of the element's own name to another URI: the
     element gets another prefix, the prefix and _0. -->
<xsl:stylesheet version="2.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform">
  <xsl:template match="/"><out><xsl:element name="q:e" namespace="urn:q1"><xsl:namespace name="q" select="'urn:q2'"/></xsl:element></out></xsl:template>
</xsl:stylesheet>
