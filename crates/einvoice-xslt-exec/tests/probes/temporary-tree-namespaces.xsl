<?xml version="1.0" encoding="UTF-8"?>
<!-- The namespaces of elements built in a temporary tree, as the stylesheets compiled by
     SchXslt build their report: nested literal result elements given the same namespaces
     (many times over, at several depths), one that declares one more, one in no namespace
     under a default namespace, an attribute in a namespace the element does not declare,
     xsl:namespace, copies of source elements. The tree, then some of its elements, copied
     to the result: each copy keeps the namespaces in scope on the element. -->
<xsl:stylesheet version="2.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform"
    xmlns:a="urn:a" xmlns:b="urn:b" xmlns="urn:default">
  <xsl:template match="/">
    <xsl:variable name="t">
      <a:report>
        <xsl:for-each select="/r/x">
          <a:rule n="{@n}">
            <a:fired x:at="{position()}" xmlns:x="urn:x"><a:text>fired</a:text></a:fired>
            <a:assert><a:text><plain>in the default namespace</plain></a:text></a:assert>
            <none xmlns=""><child/><a:back/></none>
            <a:fixup><xsl:attribute name="z:attr" namespace="urn:z">v</xsl:attribute><a:inner/></a:fixup>
            <xsl:element name="b:computed"><xsl:namespace name="n">urn:n</xsl:namespace><a:in/></xsl:element>
            <xsl:copy-of select="/r/p:x" xmlns:p="urn:p"/>
          </a:rule>
        </xsl:for-each>
        <a:rule n="last"><a:fired><a:text>again</a:text></a:fired></a:rule>
      </a:report>
    </xsl:variable>
    <out>
      <tree><xsl:copy-of select="$t"/></tree>
      <rules><xsl:sequence select="$t/a:report/a:rule[@n = '2']/node()"/></rules>
      <texts><xsl:copy-of select="$t//a:text"/></texts>
    </out>
  </xsl:template>
</xsl:stylesheet>
