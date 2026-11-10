<?xml version="1.0" encoding="UTF-8"?>
<!-- xsl:message with a terminate value that is neither yes nor no. -->
<xsl:stylesheet version="2.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform">
  <xsl:template match="/"><out><xsl:message terminate="{replace(name(/r), 'r', 'maybe')}">m</xsl:message></out></xsl:template>
</xsl:stylesheet>
