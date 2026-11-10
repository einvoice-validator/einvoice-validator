<?xml version="1.0" encoding="UTF-8"?>
<!-- generate-id(): a name of letters and digits starting with a letter, the same for a
     node, different for two, empty for no node, the context node without argument; in a
     temporary tree too. (The IDs themselves differ from Saxon's, whose numbers depend on
     the documents it read before and on its tree's numbering of nodes.) -->
<xsl:stylesheet version="2.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform">
  <xsl:template match="/">
    <out>
      <ids><xsl:value-of select="every $n in (/, /r, /r/x[2], /r/@a, /r/text()[1], /r/comment(), /r/processing-instruction(), /comment()) satisfies matches(generate-id($n), '^[A-Za-z][A-Za-z0-9]*$')"/></ids>
      <distinct><xsl:value-of select="count(distinct-values(for $n in //node() | //@* return generate-id($n))) = count(//node() | //@*)"/></distinct>
      <same><xsl:value-of select="generate-id(/r) = generate-id(/r)"/></same>
      <different><xsl:value-of select="generate-id(/r/x[1]) != generate-id(/r/x[2])"/></different>
      <empty><xsl:value-of select="generate-id(())"/>|</empty>
      <context><xsl:for-each select="/r/y"><xsl:value-of select="generate-id() = generate-id(.)"/></xsl:for-each></context>
      <temporary><xsl:variable name="t"><q><z/></q></xsl:variable><xsl:value-of select="matches(generate-id($t/q/z), '^[A-Za-z][A-Za-z0-9]*$'), generate-id($t) != generate-id(/), generate-id($t/q) != generate-id($t/q/z)"/></temporary>
    </out>
  </xsl:template>
</xsl:stylesheet>
