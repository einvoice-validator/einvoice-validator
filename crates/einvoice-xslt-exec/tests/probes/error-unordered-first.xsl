<?xml version="1.0" encoding="UTF-8"?>
<!-- Error: the first node of a path whose nodes are sorted, in document order, is known
     once every node is read, r/@b ('two') compared with a number included. -->
<xsl:stylesheet version="2.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform">
  <xsl:template match="/"><out><xsl:value-of select="name(((r/y, r)/@*[. = 3])[1])"/></out></xsl:template>
</xsl:stylesheet>
