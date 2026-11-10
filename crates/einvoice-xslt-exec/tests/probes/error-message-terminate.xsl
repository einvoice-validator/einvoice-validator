<?xml version="1.0" encoding="UTF-8"?>
<!-- xsl:message with a computed terminate="yes": the messages before, then XTMM9000. -->
<xsl:stylesheet version="2.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform">
  <xsl:template match="/"><out><xsl:message>first</xsl:message><xsl:message terminate="{concat('y', 'es')}">bye <x/></xsl:message><never/></out></xsl:template>
</xsl:stylesheet>
