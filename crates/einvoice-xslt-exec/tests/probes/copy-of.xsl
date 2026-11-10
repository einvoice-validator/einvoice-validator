<?xml version="1.0" encoding="UTF-8"?>
<!-- xsl:copy-of: attributes, elements with their namespaces, comments, processing
     instructions, a whole document, atomic values (separated by a space); the namespaces
     of copied elements, which they keep exactly whatever their new parent has (written as
     XML, only what their names need is declared); copies in a temporary tree and at the
     top of a sequence (parentless). -->
<xsl:stylesheet version="2.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform" xmlns:p="urn:p">
  <xsl:template match="/">
    <out>
      <nodes><xsl:copy-of select="/r/@b"/><xsl:copy-of select="/r/y, /r/comment(), /r/processing-instruction(), /r/p:x"/></nodes>
      <atomics><xsl:copy-of select="1, 2, 'a'"/><xsl:copy-of select="3"/></atomics>
      <document><xsl:copy-of select="/"/></document>
      <undeclare xmlns="urn:d"><xsl:copy-of select="document('ns.xml')/doc/*"/></undeclare>
      <tree>
        <xsl:variable name="t"><wrap xmlns="urn:w"><xsl:copy-of select="document('ns.xml')/doc/*"/></wrap></xsl:variable>
        <xsl:copy-of select="$t"/>
        <xsl:copy-of select="$t//*:e"/>
      </tree>
      <sequence>
        <xsl:variable name="s" as="node()*"><xsl:copy-of select="/r/x[1], /r/@a, /r/comment(), /r/processing-instruction()"/></xsl:variable>
        <xsl:value-of select="count($s), for $n in $s return name($n), count($s/..)" separator=","/>
      </sequence>
    </out>
  </xsl:template>
</xsl:stylesheet>
