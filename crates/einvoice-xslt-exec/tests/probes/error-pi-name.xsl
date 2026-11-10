<?xml version="1.0" encoding="UTF-8"?>
<!-- A processing instruction named xml (in any case). -->
<xsl:stylesheet version="2.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform">
  <xsl:template match="/"><out><xsl:processing-instruction name="{replace(name(/r), 'r', 'XmL')}">x</xsl:processing-instruction></out></xsl:template>
</xsl:stylesheet>
