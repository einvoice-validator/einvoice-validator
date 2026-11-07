<?xml version="1.0" encoding="UTF-8"?>
<!--
  Saxon-HE probe of the atomic value layer: evaluates every case of the input
  document (generated from cases.tsv and generated.tsv by the saxon-oracle example)
  and writes its result: the type and string value of each item, or the error code.

  The operands are built at run time from the lexical forms in the input document,
  so that Saxon cannot fold them at compile time: constant folding changes the
  internal representation of some values (for example the 64-bit or big form of
  an integer) and the probe must observe run-time behaviour.

  version="2.0" like the official validation stylesheets; xsl:try is accepted by
  Saxon-HE 9.9 in such a stylesheet.
-->
<xsl:stylesheet version="2.0"
    xmlns:xsl="http://www.w3.org/1999/XSL/Transform"
    xmlns:xs="http://www.w3.org/2001/XMLSchema"
    xmlns:err="http://www.w3.org/2005/xqt-errors"
    xmlns:p="urn:einvoice-xdm:probe"
    exclude-result-prefixes="xs err p">

  <xsl:output method="xml" indent="no" encoding="UTF-8"/>

  <!-- An operand: the typed value built from the lexical form, or the element
       itself for t="node" (it atomizes to xs:untypedAtomic). -->
  <xsl:function name="p:val">
    <xsl:param name="a"/>
    <xsl:variable name="s" select="string($a)"/>
    <xsl:choose>
      <xsl:when test="empty($a)"><xsl:sequence select="()"/></xsl:when>
      <xsl:when test="$a/@t = 'node'"><xsl:sequence select="$a"/></xsl:when>
      <xsl:when test="$a/@t = 'string'"><xsl:sequence select="$s"/></xsl:when>
      <xsl:when test="$a/@t = 'untyped'"><xsl:sequence select="xs:untypedAtomic($s)"/></xsl:when>
      <xsl:when test="$a/@t = 'decimal'"><xsl:sequence select="xs:decimal($s)"/></xsl:when>
      <xsl:when test="$a/@t = 'idecimal'"><xsl:sequence select="xs:decimal(xs:integer($s))"/></xsl:when>
      <xsl:when test="$a/@t = 'integer'"><xsl:sequence select="xs:integer($s)"/></xsl:when>
      <xsl:when test="$a/@t = 'double'"><xsl:sequence select="xs:double($s)"/></xsl:when>
      <xsl:when test="$a/@t = 'boolean'"><xsl:sequence select="xs:boolean($s)"/></xsl:when>
      <xsl:when test="$a/@t = 'date'"><xsl:sequence select="xs:date($s)"/></xsl:when>
      <xsl:when test="$a/@t = 'dateTime'"><xsl:sequence select="xs:dateTime($s)"/></xsl:when>
      <xsl:otherwise><xsl:sequence select="error(xs:QName('p:BADTYPE'), $a/@t)"/></xsl:otherwise>
    </xsl:choose>
  </xsl:function>

  <xsl:function name="p:type" as="xs:string">
    <xsl:param name="v"/>
    <xsl:choose>
      <xsl:when test="$v instance of xs:integer">xs:integer</xsl:when>
      <xsl:when test="$v instance of xs:decimal">xs:decimal</xsl:when>
      <xsl:when test="$v instance of xs:double">xs:double</xsl:when>
      <xsl:when test="$v instance of xs:float">xs:float</xsl:when>
      <xsl:when test="$v instance of xs:boolean">xs:boolean</xsl:when>
      <xsl:when test="$v instance of xs:untypedAtomic">xs:untypedAtomic</xsl:when>
      <xsl:when test="$v instance of xs:anyURI">xs:anyURI</xsl:when>
      <xsl:when test="$v instance of xs:string">xs:string</xsl:when>
      <xsl:when test="$v instance of xs:dateTime">xs:dateTime</xsl:when>
      <xsl:when test="$v instance of xs:date">xs:date</xsl:when>
      <xsl:otherwise>other</xsl:otherwise>
    </xsl:choose>
  </xsl:function>

  <!-- Ranges a-b,c-d of the code points (from a sequence in ascending order). -->
  <xsl:function name="p:ranges" as="xs:string">
    <xsl:param name="m" as="xs:integer*"/>
    <xsl:variable name="n" select="count($m)"/>
    <xsl:sequence select="string-join(
        for $i in 1 to $n return
          if ($i = 1 or $m[$i] ne $m[$i - 1] + 1)
          then concat($m[$i], '-',
                 (for $j in $i to $n return
                    if ($j = $n or $m[$j + 1] ne $m[$j] + 1) then $m[$j] else ())[1])
          else (), ',')"/>
  </xsl:function>

  <xsl:variable name="xml-chars" as="xs:integer*"
      select="(9, 10, 13, 32 to 55295, 57344 to 65533, 65536 to 1114111)"/>

  <xsl:function name="p:eval">
    <xsl:param name="c"/>
    <xsl:variable name="op" select="string($c/@op)"/>
    <xsl:variable name="a" select="p:val($c/a[1])"/>
    <xsl:variable name="b" select="p:val($c/a[2])"/>
    <xsl:choose>
      <xsl:when test="$op = 'add'"><xsl:sequence select="$a + $b"/></xsl:when>
      <xsl:when test="$op = 'sub'"><xsl:sequence select="$a - $b"/></xsl:when>
      <xsl:when test="$op = 'mul'"><xsl:sequence select="$a * $b"/></xsl:when>
      <xsl:when test="$op = 'div'"><xsl:sequence select="$a div $b"/></xsl:when>
      <xsl:when test="$op = 'idiv'"><xsl:sequence select="$a idiv $b"/></xsl:when>
      <xsl:when test="$op = 'mod'"><xsl:sequence select="$a mod $b"/></xsl:when>
      <xsl:when test="$op = 'neg'"><xsl:sequence select="- $a"/></xsl:when>
      <xsl:when test="$op = 'g-eq'"><xsl:sequence select="$a = $b"/></xsl:when>
      <xsl:when test="$op = 'g-ne'"><xsl:sequence select="$a != $b"/></xsl:when>
      <xsl:when test="$op = 'g-lt'"><xsl:sequence select="$a &lt; $b"/></xsl:when>
      <xsl:when test="$op = 'g-le'"><xsl:sequence select="$a &lt;= $b"/></xsl:when>
      <xsl:when test="$op = 'g-gt'"><xsl:sequence select="$a &gt; $b"/></xsl:when>
      <xsl:when test="$op = 'g-ge'"><xsl:sequence select="$a &gt;= $b"/></xsl:when>
      <xsl:when test="$op = 'v-eq'"><xsl:sequence select="$a eq $b"/></xsl:when>
      <xsl:when test="$op = 'v-ne'"><xsl:sequence select="$a ne $b"/></xsl:when>
      <xsl:when test="$op = 'v-lt'"><xsl:sequence select="$a lt $b"/></xsl:when>
      <xsl:when test="$op = 'v-le'"><xsl:sequence select="$a le $b"/></xsl:when>
      <xsl:when test="$op = 'v-gt'"><xsl:sequence select="$a gt $b"/></xsl:when>
      <xsl:when test="$op = 'v-ge'"><xsl:sequence select="$a ge $b"/></xsl:when>
      <xsl:when test="$op = 'cast-string'"><xsl:sequence select="$a cast as xs:string"/></xsl:when>
      <xsl:when test="$op = 'cast-untypedAtomic'"><xsl:sequence select="$a cast as xs:untypedAtomic"/></xsl:when>
      <xsl:when test="$op = 'cast-boolean'"><xsl:sequence select="$a cast as xs:boolean"/></xsl:when>
      <xsl:when test="$op = 'cast-integer'"><xsl:sequence select="$a cast as xs:integer"/></xsl:when>
      <xsl:when test="$op = 'cast-decimal'"><xsl:sequence select="$a cast as xs:decimal"/></xsl:when>
      <xsl:when test="$op = 'cast-double'"><xsl:sequence select="$a cast as xs:double"/></xsl:when>
      <xsl:when test="$op = 'cast-date'"><xsl:sequence select="$a cast as xs:date"/></xsl:when>
      <xsl:when test="$op = 'cast-dateTime'"><xsl:sequence select="$a cast as xs:dateTime"/></xsl:when>
      <xsl:when test="$op = 'castable-string'"><xsl:sequence select="$a castable as xs:string"/></xsl:when>
      <xsl:when test="$op = 'castable-boolean'"><xsl:sequence select="$a castable as xs:boolean"/></xsl:when>
      <xsl:when test="$op = 'castable-integer'"><xsl:sequence select="$a castable as xs:integer"/></xsl:when>
      <xsl:when test="$op = 'castable-decimal'"><xsl:sequence select="$a castable as xs:decimal"/></xsl:when>
      <xsl:when test="$op = 'castable-double'"><xsl:sequence select="$a castable as xs:double"/></xsl:when>
      <xsl:when test="$op = 'castable-date'"><xsl:sequence select="$a castable as xs:date"/></xsl:when>
      <xsl:when test="$op = 'castable-dateTime'"><xsl:sequence select="$a castable as xs:dateTime"/></xsl:when>
      <xsl:when test="$op = 'string'"><xsl:sequence select="string($a)"/></xsl:when>
      <xsl:when test="$op = 'number'"><xsl:sequence select="number($a)"/></xsl:when>
      <xsl:when test="$op = 'boolean'"><xsl:sequence select="boolean($a)"/></xsl:when>
      <xsl:when test="$op = 'round'"><xsl:sequence select="round($a)"/></xsl:when>
      <xsl:when test="$op = 'floor'"><xsl:sequence select="floor($a)"/></xsl:when>
      <xsl:when test="$op = 'ceiling'"><xsl:sequence select="ceiling($a)"/></xsl:when>
      <xsl:when test="$op = 'abs'"><xsl:sequence select="abs($a)"/></xsl:when>
      <xsl:when test="$op = 'sum'"><xsl:sequence select="sum(for $x in $c/a return p:val($x))"/></xsl:when>
      <xsl:when test="$op = 'matches'"><xsl:sequence select="matches($a, $b)"/></xsl:when>
      <xsl:when test="$op = 'replace'"><xsl:sequence select="replace($a, $b, p:val($c/a[3]))"/></xsl:when>
      <xsl:when test="$op = 'tokenize'"><xsl:sequence select="tokenize($a, $b)"/></xsl:when>
      <xsl:when test="$op = 'cps-regex'">
        <xsl:sequence select="p:ranges($xml-chars[matches(codepoints-to-string(.), $a)])"/>
      </xsl:when>
      <xsl:when test="$op = 'int-digits'">
        <!-- 16 characters plus one: Saxon's long integer path (BigInteger). -->
        <xsl:sequence select="string-join(
            for $cp in (32 to 55295, 57344 to 65533)
            return
              if (concat('1111111111111111', codepoints-to-string($cp)) castable as xs:integer)
              then concat($cp, ':', xs:integer(concat('1000000000000000', codepoints-to-string($cp))) mod 10)
              else (), ',')"/>
      </xsl:when>
      <xsl:otherwise><xsl:sequence select="error(xs:QName('p:BADOP'), $op)"/></xsl:otherwise>
    </xsl:choose>
  </xsl:function>

  <xsl:template match="/">
    <results>
      <xsl:for-each select="/cases/c">
        <r id="{@id}">
          <xsl:try>
            <xsl:variable name="result" select="p:eval(.)"/>
            <xsl:for-each select="$result">
              <v t="{p:type(.)}"><xsl:value-of select="."/></v>
            </xsl:for-each>
            <xsl:catch>
              <e code="{local-name-from-QName($err:code)}"/>
            </xsl:catch>
          </xsl:try>
        </r>
      </xsl:for-each>
    </results>
  </xsl:template>

</xsl:stylesheet>
