<?xml version="1.0" encoding="UTF-8"?>
<!-- Error: count() reads every node of a path whose nodes are sorted, r/@b ('two')
     compared with a number included, even when the node it reads first matches. -->
<xsl:stylesheet version="2.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform">
  <xsl:template match="/"><out><xsl:value-of select="count((r/y, r)/@*[. = 3])"/></out></xsl:template>
</xsl:stylesheet>
