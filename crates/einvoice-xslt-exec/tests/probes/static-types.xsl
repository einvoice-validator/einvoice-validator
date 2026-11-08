<?xml version="1.0" encoding="UTF-8"?>
<!-- The static types Saxon-HE gives variable and parameter references, from their select
     expressions and `as` declarations, where they change a result: `xs:decimal` of a value
     statically xs:decimal is the value itself (an xs:integer stays one), of a value
     statically xs:integer strips its trailing zeros, and `div` of values statically
     xs:decimal is a decimal division. count(/r/x) is 3. -->
<xsl:stylesheet version="2.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform"
    xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:f="urn:f">
  <xsl:output indent="no"/>
  <xsl:param name="p-dec" as="xs:decimal" select="300"/>
  <xsl:param name="p-int" as="xs:integer" select="300"/>
  <xsl:param name="p-none" select="300"/>
  <xsl:param name="p-dec-expr" as="xs:decimal" select="count(/r/x) * 100"/>
  <xsl:param name="p-none-expr" select="count(/r/x) div 1 - 4"/>
  <xsl:variable name="g-div" select="count(/r/x) * 1000 div 10"/>
  <xsl:variable name="g-int" select="count(/r/x) * 100"/>
  <xsl:variable name="g-neg" select="count(/r/x) div 1 - 4"/>
  <xsl:variable name="g-lit" select="300"/>
  <xsl:variable name="g-lit-seq" select="(300, 'a')"/>
  <xsl:variable name="g-folded" select="(300, 'a')[1]"/>
  <xsl:variable name="g-folded-dec" select="(300, 2.5)[1]"/>
  <xsl:variable name="g-count" select="count(/r/x)"/>
  <xsl:variable name="g-if" select="if (count(/r/x) = 3) then 300 else 1.5"/>
  <xsl:variable name="g-fwd" select="$g-late-count"/>
  <xsl:variable name="g-bwd" select="$g-count"/>
  <xsl:variable name="g-fn" select="f:result-dec(/r/x)"/>
  <xsl:template name="early">
    <xsl:variable name="l-fwd" as="xs:decimal" select="$g-late-count"/>
    <xsl:variable name="l-bwd" as="xs:decimal" select="$g-count"/>
    <c n="l-fwd"><xsl:value-of select="xs:decimal($l-fwd) instance of xs:integer, 1.1 div xs:decimal($l-fwd * 100)"/></c>
    <c n="l-bwd"><xsl:value-of select="xs:decimal($l-bwd) instance of xs:integer, 1.1 div xs:decimal($l-bwd * 100)"/></c>
  </xsl:template>
  <xsl:variable name="g-late-count" select="count(/r/x)"/>
  <xsl:variable name="g-as-dec" as="xs:decimal" select="count(/r/x) * 100"/>
  <xsl:variable name="g-as-dec-any" as="xs:decimal" select="f:any(300)"/>
  <xsl:variable name="g-as-int-any" as="xs:integer" select="f:any(300)"/>
  <xsl:variable name="g-as-dec-count" as="xs:decimal" select="count(/r/x)"/>
  <xsl:variable name="g-as-dec-lit" as="xs:decimal" select="300"/>
  <xsl:variable name="g-any" select="f:any(count(/r/x) div 1 - 4)"/>
  <xsl:variable name="g-ref" select="$g-neg"/>
  <xsl:variable name="g-ref-int" select="$g-int"/>
  <xsl:variable name="g-tree">300</xsl:variable>
  <xsl:function name="f:any">
    <xsl:param name="x"/>
    <xsl:sequence select="$x"/>
  </xsl:function>
  <xsl:function name="f:dec">
    <xsl:param name="x" as="xs:decimal"/>
    <xsl:sequence select="xs:decimal($x) instance of xs:integer, 1.1 div xs:decimal($x), ($x - 301) div 300"/>
  </xsl:function>
  <xsl:function name="f:int">
    <xsl:param name="x" as="xs:integer"/>
    <xsl:sequence select="1.1 div xs:decimal($x)"/>
  </xsl:function>
  <xsl:function name="f:none">
    <xsl:param name="x"/>
    <xsl:sequence select="xs:decimal($x) instance of xs:integer, 1.1 div xs:decimal($x), ($x - 301) div 300"/>
  </xsl:function>
  <xsl:function name="f:local">
    <xsl:param name="x"/>
    <xsl:variable name="v" select="count($x) * 100"/>
    <xsl:variable name="w" select="count($x) div 1 - 4"/>
    <xsl:sequence select="1.1 div xs:decimal($v), $w div 300"/>
  </xsl:function>
  <xsl:function name="f:result-dec" as="xs:decimal">
    <xsl:param name="x"/>
    <xsl:sequence select="count($x) * 100"/>
  </xsl:function>
  <xsl:function name="f:result-int" as="xs:integer">
    <xsl:param name="x"/>
    <xsl:sequence select="count($x) * 100"/>
  </xsl:function>
  <xsl:function name="f:result-neg" as="xs:decimal">
    <xsl:param name="x"/>
    <xsl:sequence select="count($x) - 4"/>
  </xsl:function>
  <xsl:template match="/">
    <out>
      <params>
        <c n="p-dec"><xsl:value-of select="xs:decimal($p-dec) instance of xs:integer, 1.1 div xs:decimal($p-dec), ($p-dec - 301) div 300"/></c>
        <c n="p-int"><xsl:value-of select="1.1 div xs:decimal($p-int)"/></c>
        <c n="p-none"><xsl:value-of select="xs:decimal($p-none) instance of xs:integer, 1.1 div xs:decimal($p-none), ($p-none - 301) div 300"/></c>
        <c n="p-dec-expr"><xsl:value-of select="xs:decimal($p-dec-expr) instance of xs:integer, 1.1 div xs:decimal($p-dec-expr)"/></c>
        <c n="p-none-expr"><xsl:value-of select="$p-none-expr div 300"/></c>
      </params>
      <globals>
        <c n="g-div"><xsl:value-of select="xs:decimal($g-div) instance of xs:integer, 1.1 div xs:decimal($g-div)"/></c>
        <c n="g-int"><xsl:value-of select="1.1 div xs:decimal($g-int)"/></c>
        <c n="g-neg"><xsl:value-of select="$g-neg div 300"/></c>
        <c n="g-lit"><xsl:value-of select="xs:decimal($g-lit) instance of xs:integer, 1.1 div xs:decimal($g-lit), ($g-lit - 301) div 300"/></c>
        <c n="g-lit-seq"><xsl:value-of select="1.1 div xs:decimal($g-lit-seq[1]), ($g-lit-seq[1] - 301) div 300"/></c>
        <c n="g-folded"><xsl:value-of select="1.1 div xs:decimal($g-folded), ($g-folded - 301) div 300"/></c>
        <c n="g-folded-dec"><xsl:value-of select="xs:decimal($g-folded-dec) instance of xs:integer, 1.1 div xs:decimal($g-folded-dec), ($g-folded-dec - 301) div 300"/></c>
        <c n="g-count"><xsl:value-of select="1.1 div xs:decimal($g-count * 100)"/></c>
        <c n="g-if"><xsl:value-of select="xs:decimal($g-if) instance of xs:integer, ($g-if - 301) div 300"/></c>
        <c n="g-as-dec"><xsl:value-of select="xs:decimal($g-as-dec) instance of xs:integer, 1.1 div xs:decimal($g-as-dec), ($g-as-dec - 301) div 300"/></c>
        <c n="g-as-dec-any"><xsl:value-of select="xs:decimal($g-as-dec-any) instance of xs:integer, 1.1 div xs:decimal($g-as-dec-any), ($g-as-dec-any - 301) div 300"/></c>
        <c n="g-as-int-any"><xsl:value-of select="1.1 div xs:decimal($g-as-int-any)"/></c>
        <c n="g-as-dec-count"><xsl:value-of select="xs:decimal($g-as-dec-count) instance of xs:integer, 1.1 div xs:decimal($g-as-dec-count * 100)"/></c>
        <c n="g-as-dec-lit"><xsl:value-of select="xs:decimal($g-as-dec-lit) instance of xs:integer, 1.1 div xs:decimal($g-as-dec-lit), ($g-as-dec-lit - 301) div 300"/></c>
        <c n="g-any"><xsl:value-of select="$g-any div 300"/></c>
        <c n="g-ref"><xsl:value-of select="$g-ref div 300"/></c>
        <c n="g-ref-int"><xsl:value-of select="1.1 div xs:decimal($g-ref-int)"/></c>
        <c n="g-tree"><xsl:value-of select="1.1 div xs:decimal($g-tree)"/></c>
        <c n="g-fwd"><xsl:value-of select="1.1 div xs:decimal($g-fwd * 100)"/></c>
        <c n="g-bwd"><xsl:value-of select="1.1 div xs:decimal($g-bwd * 100)"/></c>
        <c n="g-fn"><xsl:value-of select="xs:decimal($g-fn) instance of xs:integer, 1.1 div xs:decimal($g-fn)"/></c>
        <xsl:call-template name="early"/>
      </globals>
      <locals>
        <xsl:variable name="l-div" select="count(/r/x) * 1000 div 10"/>
        <c n="l-div"><xsl:value-of select="xs:decimal($l-div) instance of xs:integer, 1.1 div xs:decimal($l-div)"/></c>
        <xsl:variable name="l-div-once" select="count(/r/x) * 1000 div 10"/>
        <c n="l-div-once"><xsl:value-of select="xs:decimal($l-div-once) instance of xs:integer"/></c>
        <xsl:variable name="l-int" select="count(/r/x) * 100"/>
        <c n="l-int"><xsl:value-of select="1.1 div xs:decimal($l-int)"/></c>
        <xsl:variable name="l-neg" select="count(/r/x) div 1 - 4"/>
        <c n="l-neg"><xsl:value-of select="$l-neg div 300, $l-neg * 1"/></c>
        <xsl:variable name="l-lit" select="300"/>
        <c n="l-lit"><xsl:value-of select="xs:decimal($l-lit) instance of xs:integer, 1.1 div xs:decimal($l-lit), ($l-lit - 301) div 300"/></c>
        <xsl:variable name="l-folded" select="(300, 'a')[1]"/>
        <c n="l-folded"><xsl:value-of select="1.1 div xs:decimal($l-folded), ($l-folded - 301) div 300"/></c>
        <xsl:variable name="l-folded-dec" select="(300, 2.5)[1]"/>
        <c n="l-folded-dec"><xsl:value-of select="xs:decimal($l-folded-dec) instance of xs:integer, 1.1 div xs:decimal($l-folded-dec), ($l-folded-dec - 301) div 300"/></c>
        <xsl:variable name="l-if" select="if (count(/r/x) = 3) then 300 else 1.5"/>
        <c n="l-if"><xsl:value-of select="xs:decimal($l-if) instance of xs:integer, ($l-if - 301) div 300"/></c>
        <xsl:variable name="l-as-dec" as="xs:decimal" select="count(/r/x) * 100"/>
        <c n="l-as-dec"><xsl:value-of select="xs:decimal($l-as-dec) instance of xs:integer, 1.1 div xs:decimal($l-as-dec), ($l-as-dec - 301) div 300"/></c>
        <xsl:variable name="l-as-dec-any" as="xs:decimal" select="f:any(300)"/>
        <c n="l-as-dec-any"><xsl:value-of select="xs:decimal($l-as-dec-any) instance of xs:integer, 1.1 div xs:decimal($l-as-dec-any), ($l-as-dec-any - 301) div 300"/></c>
        <xsl:variable name="l-as-int-any" as="xs:integer" select="f:any(300)"/>
        <c n="l-as-int-any"><xsl:value-of select="1.1 div xs:decimal($l-as-int-any)"/></c>
        <xsl:variable name="l-as-dec-lit" as="xs:decimal" select="300"/>
        <c n="l-as-dec-lit"><xsl:value-of select="xs:decimal($l-as-dec-lit) instance of xs:integer, 1.1 div xs:decimal($l-as-dec-lit), ($l-as-dec-lit - 301) div 300"/></c>
        <xsl:variable name="l-any" select="f:any(count(/r/x) div 1 - 4)"/>
        <c n="l-any"><xsl:value-of select="$l-any div 300"/></c>
        <xsl:variable name="l-ref" select="$l-neg"/>
        <c n="l-ref"><xsl:value-of select="$l-ref div 300"/></c>
        <xsl:variable name="l-ref-global" select="$g-neg"/>
        <c n="l-ref-global"><xsl:value-of select="$l-ref-global div 300"/></c>
        <xsl:variable name="l-tree">300</xsl:variable>
        <c n="l-tree"><xsl:value-of select="1.1 div xs:decimal($l-tree)"/></c>
        <xsl:variable name="l-as-dec-tree" as="xs:decimal"><xsl:sequence select="/r/amounts/amount[2]"/></xsl:variable>
        <c n="l-as-dec-tree"><xsl:value-of select="xs:decimal($l-as-dec-tree) instance of xs:integer, 1.1 div xs:decimal($l-as-dec-tree)"/></c>
        <xsl:variable name="l-a-int" select="count(/r/x)"/>
        <xsl:variable name="l-b-int" as="xs:decimal" select="$l-a-int"/>
        <c n="l-b-int"><xsl:value-of select="xs:decimal($l-b-int) instance of xs:integer, 1.1 div xs:decimal($l-b-int * 100)"/></c>
        <xsl:variable name="l-a-arith" select="count(/r/x) * 100"/>
        <xsl:variable name="l-b-arith" as="xs:decimal" select="$l-a-arith"/>
        <c n="l-b-arith"><xsl:value-of select="xs:decimal($l-b-arith) instance of xs:integer, 1.1 div xs:decimal($l-b-arith), ($l-b-arith - 301) div 300"/></c>
        <xsl:variable name="l-cast" select="xs:decimal(count(/r/x) * 100 div 1)"/>
        <c n="l-cast"><xsl:value-of select="xs:decimal($l-cast) instance of xs:integer, ($l-cast - 301) div 300"/></c>
        <xsl:variable name="l-lit-arith" select="3000 div 10"/>
        <c n="l-lit-arith"><xsl:value-of select="xs:decimal($l-lit-arith) instance of xs:integer, 1.1 div xs:decimal($l-lit-arith)"/></c>
        <xsl:variable name="l-fn" select="f:result-dec(/r/x)"/>
        <c n="l-fn"><xsl:value-of select="xs:decimal($l-fn) instance of xs:integer, 1.1 div xs:decimal($l-fn)"/></c>
        <xsl:variable name="l-seq" select="(300, 'a')"/>
        <c n="l-seq"><xsl:value-of select="1.1 div xs:decimal($l-seq[1]), ($l-seq[1] - 301) div 300"/></c>
        <xsl:for-each select="/r/x[1]">
          <xsl:variable name="l-in" select="count(../x) div 1 - 4"/>
          <c n="l-in-for-each"><xsl:value-of select="$l-in div 300, $l-neg div 300"/></c>
        </xsl:for-each>
      </locals>
      <template-params>
        <xsl:call-template name="params">
          <xsl:with-param name="t-dec" select="count(/r/x) * 100"/>
          <xsl:with-param name="t-int" select="count(/r/x) * 100"/>
          <xsl:with-param name="t-none" select="count(/r/x) * 100"/>
        </xsl:call-template>
      </template-params>
      <functions>
        <c n="f-dec"><xsl:value-of select="f:dec(count(/r/x) * 100)"/></c>
        <c n="f-int"><xsl:value-of select="f:int(count(/r/x) * 100)"/></c>
        <c n="f-none"><xsl:value-of select="f:none(count(/r/x) * 100)"/></c>
        <c n="f-local"><xsl:value-of select="f:local(/r/x)"/></c>
        <c n="f-result-dec"><xsl:value-of select="xs:decimal(f:result-dec(/r/x)) instance of xs:integer, 1.1 div xs:decimal(f:result-dec(/r/x))"/></c>
        <c n="f-result-int"><xsl:value-of select="1.1 div xs:decimal(f:result-int(/r/x))"/></c>
        <c n="f-result-neg"><xsl:value-of select="f:result-neg(/r/x) div 300"/></c>
      </functions>
    </out>
  </xsl:template>
  <xsl:template name="params">
    <xsl:param name="t-dec" as="xs:decimal"/>
    <xsl:param name="t-int" as="xs:integer"/>
    <xsl:param name="t-none"/>
    <xsl:param name="t-default" select="count(/r/x) div 1 - 4"/>
    <xsl:param name="t-default-dec" as="xs:decimal" select="count(/r/x) div 1 - 4"/>
    <c n="t-dec"><xsl:value-of select="xs:decimal($t-dec) instance of xs:integer, 1.1 div xs:decimal($t-dec), ($t-dec - 301) div 300"/></c>
    <c n="t-int"><xsl:value-of select="1.1 div xs:decimal($t-int)"/></c>
    <c n="t-none"><xsl:value-of select="xs:decimal($t-none) instance of xs:integer, 1.1 div xs:decimal($t-none), ($t-none - 301) div 300"/></c>
    <c n="t-default"><xsl:value-of select="$t-default div 300"/></c>
    <c n="t-default-dec"><xsl:value-of select="$t-default-dec div 300"/></c>
  </xsl:template>
</xsl:stylesheet>
