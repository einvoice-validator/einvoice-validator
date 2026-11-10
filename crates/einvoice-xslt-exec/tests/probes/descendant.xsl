<?xml version="1.0" encoding="UTF-8"?>
<!-- The descendant axis, with predicates and positions. -->
<xsl:stylesheet version="2.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform">
  <xsl:template match="/">
    <out><xsl:value-of select="count(/descendant::x), count(/r/descendant::text()), name(/descendant::*[3]), count(//y/descendant::node()), string(/r/descendant::amount[last()]), count(/r/x[1]/descendant::*), name(descendant::*[@z][1])" separator="|"/></out>
  </xsl:template>
</xsl:stylesheet>
