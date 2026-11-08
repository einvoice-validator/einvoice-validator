<?xml version="1.0" encoding="UTF-8"?>
<!-- Nodes constructed in a sequence (an xsl:function body, a variable or parameter with
     `as`), as the stylesheets compiled by SchXslt construct them: parentless elements,
     attributes, text nodes and comments, their content, their namespaces, their copies
     into the result, and their atomization when the sequence type asks for atomic
     values. -->
<xsl:stylesheet version="2.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform"
                xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:p="urn:p" xmlns:f="urn:f"
                xmlns:s="urn:s">
  <xsl:output indent="no"/>
  <xsl:template name="report">
    <s:document>
      <s:pattern id="p1">
        <xsl:attribute name="documents" select="'d'"/>
        <s:active id="a1"/>
      </s:pattern>
      <s:pattern id="p2"><s:active id="a2"/></s:pattern>
      <xsl:apply-templates select="/r/x" mode="rules"/>
    </s:document>
  </xsl:template>
  <xsl:template match="x" mode="rules">
    <s:rule pattern="{if (@n = '2') then 'p2' else 'p1'}">
      <s:fired context="x">
        <xsl:variable name="uri" as="xs:anyURI?" select="document-uri()"/>
        <xsl:if test="exists($uri)"><xsl:attribute name="document" select="$uri"/></xsl:if>
        <xsl:attribute name="n" select="@n"/>
      </s:fired>
      <xsl:comment>c<xsl:value-of select="@n"/></xsl:comment>
      <xsl:value-of select="concat('t', @n)"/>
    </s:rule>
  </xsl:template>
  <xsl:function name="f:location" as="xs:string">
    <xsl:param name="node" as="node()"/>
    <xsl:variable name="segments" as="xs:string*">
      <xsl:for-each select="$node/ancestor-or-self::node()">
        <xsl:variable name="position"><xsl:number level="single"/></xsl:variable>
        <xsl:choose>
          <xsl:when test=". instance of element()">
            <xsl:value-of select="concat('Q{', namespace-uri(.), '}', local-name(.), '[', $position, ']')"/>
          </xsl:when>
          <xsl:when test=". instance of attribute()">
            <xsl:value-of select="concat('@Q{', namespace-uri(.), '}', local-name(.))"/>
          </xsl:when>
          <xsl:when test=". instance of text()">
            <xsl:value-of select="concat('text()[', $position, ']')"/>
          </xsl:when>
          <xsl:otherwise/>
        </xsl:choose>
      </xsl:for-each>
    </xsl:variable>
    <xsl:value-of select="concat('/', string-join($segments, '/'))"/>
  </xsl:function>
  <xsl:function name="f:strings" as="xs:string*">
    <xsl:value-of select="''"/>
    <xsl:text>a</xsl:text>
    <xsl:text>b</xsl:text>
    <xsl:value-of select="()"/>
    <xsl:value-of select="1, 2" separator="-"/>
    <xsl:sequence select="'c'"/>
  </xsl:function>
  <xsl:function name="f:nodes" as="node()*">
    <xsl:param name="doc" as="node()"/>
    <xsl:value-of select="''"/>
    <xsl:text>a</xsl:text>
    <xsl:text/>
    <xsl:text>b</xsl:text>
    <xsl:attribute name="at">v</xsl:attribute>
    <xsl:attribute name="p:at" select="1, 2"/>
    <xsl:comment>k</xsl:comment>
    <e a="1"><xsl:sequence select="2, 3"/><xsl:value-of select="''"/>t</e>
    <xsl:sequence select="$doc/r/y"/>
  </xsl:function>
  <xsl:function name="f:element" as="element()">
    <xsl:param name="n" as="xs:string"/>
    <f:e n="{$n}"><xsl:value-of select="$n"/></f:e>
  </xsl:function>
  <xsl:template match="/">
    <xsl:variable name="metadata" as="element()?">
      <s:metadata xmlns:dct="http://purl.org/dc/terms/" xmlns:skos="http://www.w3.org/2004/02/skos/core#">
        <dct:creator><dct:Agent><skos:prefLabel><xsl:value-of separator="/" select="(system-property('xsl:product-name'), system-property('xsl:product-version'))"/></skos:prefLabel></dct:Agent></dct:creator>
        <dct:source>
          <p:Description xmlns:dc="http://purl.org/dc/elements/1.1/">
            <named xmlns="urn:default">x<inner/></named>
          </p:Description>
        </dct:source>
      </s:metadata>
    </xsl:variable>
    <xsl:variable name="report" as="element(s:report)">
      <s:report><xsl:call-template name="report"/></s:report>
    </xsl:variable>
    <xsl:variable name="joined" as="node()*">
      <xsl:sequence select="$metadata"/>
      <xsl:for-each select="$report/s:document">
        <xsl:for-each select="s:pattern">
          <xsl:sequence select="node()"/>
          <xsl:sequence select="../s:rule[@pattern = current()/@id]/node()"/>
        </xsl:for-each>
      </xsl:for-each>
    </xsl:variable>
    <out xmlns:o="urn:o">
      <joined><xsl:sequence select="$joined"/></joined>
      <count><xsl:value-of select="count($joined), count($report/*), count($report/s:document/s:pattern/@*)"/></count>
      <names><xsl:value-of select="for $n in $joined return name($n)" separator=","/></names>
      <parents><xsl:value-of select="count($metadata/..), count($report/..), name(root($report)), count(root($metadata/*[1])/..)"/></parents>
      <document-uri><xsl:value-of select="count(document-uri($report)), count(document-uri($report/s:document))"/></document-uri>
      <string-values><xsl:value-of select="string($report), '|', string($metadata)"/></string-values>
      <locations><xsl:value-of select="f:location(/r/x[2]), f:location(/r/x[2]/@n), f:location(/r/y/text()[2]), f:location($report/s:document/s:rule[1])" separator=" "/></locations>
      <strings><xsl:value-of select="count(f:strings()), string-join(f:strings(), ',')"/></strings>
      <nodes-count><xsl:value-of select="count(f:nodes(/)), for $n in f:nodes(/) return concat(name($n), '=', string($n))" separator=","/></nodes-count>
      <nodes-kinds><xsl:value-of select="for $n in f:nodes(/) return if ($n instance of element()) then 'e' else if ($n instance of attribute()) then 'a' else if ($n instance of text()) then 't' else if ($n instance of comment()) then 'c' else '?'" separator=""/></nodes-kinds>
      <nodes><xsl:sequence select="f:nodes(/)[not(self::text())]"/></nodes>
      <nodes-text><xsl:sequence select="f:nodes(/)[self::text()]"/></nodes-text>
      <element><xsl:sequence select="f:element('v'), f:element('w')"/></element>
      <element-parent><xsl:value-of select="count(f:element('v')/..), f:element('v')/@n, f:element('v')/text()"/></element-parent>
      <empty-element as="node()*">
        <xsl:variable name="e" as="element()"><e/></xsl:variable>
        <xsl:value-of select="count($e/node()), count($e/@*), name($e)"/>
      </empty-element>
    </out>
  </xsl:template>
</xsl:stylesheet>
