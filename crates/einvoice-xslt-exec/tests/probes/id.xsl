<?xml version="1.0" encoding="UTF-8"?>
<!-- id(): xml:id attributes of a document read by document() (ids.xml), whitespace in
     their value and in the argument, several IDs, a duplicate, an id attribute that is not
     an ID, the two-argument form; none in the source document. -->
<xsl:stylesheet version="2.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform" xmlns:p="urn:p">
  <xsl:variable name="d" select="document('ids.xml')"/>
  <xsl:template match="/">
    <out>
      <xsl:for-each select="$d">
        <a n="{count(id('a'))}" b="{count(id('b c'))}" c="{count(id(' a '))}" d="{count(id('x'))}"
           e="{count(id('dup'))}" f="{id('dup')/@k}" g="{count(id(('a', 'b', 'a')))}" h="{name(id('pf'))}"/>
      </xsl:for-each>
      <two n="{count(id('b', $d/doc/e[3]))}"/>
      <source n="{count(id('a'))}"/>
      <order><xsl:value-of select="for $e in $d/id('c b') return string($e/@xml:id)"/></order>
    </out>
  </xsl:template>
</xsl:stylesheet>
