<?xml version="1.0" encoding="UTF-8"?>
<!-- When Saxon-HE 12.10 evaluates local variables and parameters, seen through values that
     raise an error (xs:decimal('two')) where nothing reads them: a variable referred to
     once, not in a loop (inlined), twice, in a loop, a temporary tree, a sequence with
     `as`, template parameter defaults, call-template parameters, and Saxon's learning,
     which evaluates a binding where it is bound once it has been evaluated 20 times with
     its value read each time (19 here, or once unread), and in a function body after 20
     evaluations (19 here), and a variable whose references Saxon counts again after
     inlining the variable right after it. None raises an error. -->
<xsl:stylesheet version="2.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform"
                xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:f="urn:f">
  <xsl:output indent="no"/>
  <xsl:variable name="no" select="/r/@a = '9'"/>
  <xsl:function name="f:chain" as="xs:boolean">
    <xsl:param name="s" as="xs:string?"/>
    <xsl:variable name="ok" select="matches($s, '^[0-9]+$')"/>
    <xsl:variable name="m" select="xs:decimal(substring($s, 1, 2))"/>
    <xsl:variable name="d" select="xs:decimal(substring($s, 3, 2))"/>
    <xsl:variable name="max" select="if ($m = (1, 3)) then 31 else if ($m = 2) then 28 else 0"/>
    <xsl:sequence select="$ok and $d le $max"/>
  </xsl:function>
  <xsl:function name="f:twice" as="xs:boolean">
    <xsl:param name="s" as="xs:string"/>
    <xsl:variable name="m" select="xs:decimal(substring($s, 2))"/>
    <xsl:sequence select="starts-with($s, 'y') and ($m = 1 or $m = 2)"/>
  </xsl:function>
  <xsl:function name="f:recounted" as="xs:boolean">
    <xsl:param name="date" as="xs:string?"/>
    <xsl:variable name="year" select="xs:decimal(substring($date, 1, 4))"/>
    <xsl:variable name="isLeapYear" select="($year mod 4 = 0 and $year mod 100 != 0) or ($year mod 400 = 0)"/>
    <xsl:variable name="maxDay" select="if (string-length($date) = 3) then 31 else if ($isLeapYear) then 29 else 0"/>
    <xsl:sequence select="matches($date, '^20') and 1 le $maxDay"/>
  </xsl:function>
  <xsl:function name="f:arg" as="xs:boolean">
    <xsl:param name="s"/>
    <xsl:param name="b" as="xs:boolean"/>
    <xsl:sequence select="$b and $s = 1"/>
  </xsl:function>
  <xsl:template match="/">
    <xsl:variable name="d" select="xs:decimal(/r/@b)"/>
    <xsl:variable name="e" select="$d + 1"/>
    <xsl:variable name="tree"><xsl:value-of select="xs:decimal(/r/@b)"/></xsl:variable>
    <xsl:variable name="typed" as="xs:decimal"><xsl:sequence select="xs:decimal(/r/@b)"/></xsl:variable>
    <out>
      <once><xsl:if test="$no and $d > 0">x</xsl:if></once>
      <twice><xsl:if test="$no and $d > 0 and $d &lt; 9">x</xsl:if><xsl:if test="$no and $d > 0">x</xsl:if></twice>
      <in-loop><xsl:for-each select="/r/x"><xsl:if test="$no and $d > 0">x</xsl:if></xsl:for-each>|<xsl:value-of select="count(/r/x[$no and . = $d]), count(/r/x[$no]/string($d))"/></in-loop>
      <in-quantified><xsl:value-of select="some $x in /r/x satisfies ($no and $x = $d), count(for $x in /r/nothing return $d)"/></in-quantified>
      <not-reached><xsl:choose><xsl:when test="not($no)">a</xsl:when><xsl:when test="$d > 0">b</xsl:when></xsl:choose><xsl:if test="$no"><e a="{$d}"/><xsl:value-of select="$d"/></xsl:if></not-reached>
      <chained><xsl:if test="$no and $e > 0 and $e &lt; 9">x</xsl:if></chained>
      <tree><xsl:if test="$no and ($tree = 1 or $tree = 2)">x</xsl:if><xsl:if test="$no and $typed = 1">x</xsl:if></tree>
      <filtered><xsl:if test="$no and $d[1] > 0">x</xsl:if></filtered>
      <function><xsl:value-of select="f:chain('-1x-'), f:chain('x-'), f:arg(xs:decimal(/r/@b), false()), f:recounted(concat('08/02/2018', /r/@b))"/></function>
      <function-learning><xsl:value-of select="for $i in (1 to 19) return f:twice(concat('y', /r/@a)), f:twice(concat('n', /r/@b))"/></function-learning>
      <param><xsl:apply-templates select="/r" mode="param"/></param>
      <call><xsl:call-template name="call"><xsl:with-param name="p" select="xs:decimal(/r/@b)"/></xsl:call-template></call>
      <learning-19><xsl:for-each select="1 to 20"><xsl:variable name="m" select="xs:decimal(if (. = 20) then 'x' else '1')"/><xsl:if test="(. != 20) and ($m = 1 or $m = 2)">x</xsl:if>.</xsl:for-each></learning-19>
      <learning-unread><xsl:for-each select="1 to 30"><xsl:variable name="m" select="xs:decimal(if (. = 30) then 'x' else '1')"/><xsl:if test="(. != 30 and . != 3) and ($m = 1 or $m = 2)">x</xsl:if>.</xsl:for-each></learning-unread>
      <learning-never-read><xsl:for-each select="1 to 25"><xsl:variable name="m" select="xs:decimal(if (. = 25) then 'x' else '1')"/><xsl:if test="$no and ($m = 1 or $m = 2)">x</xsl:if>.</xsl:for-each></learning-never-read>
      <param-learning><xsl:apply-templates select="for $i in 1 to 20 return /r/x[1]" mode="learning"/><xsl:apply-templates select="/r/y" mode="learning"/></param-learning>
    </out>
  </xsl:template>
  <xsl:template match="r" mode="param"><xsl:param name="p" select="xs:decimal(@b)"/><xsl:if test="$no and $p = 1">x</xsl:if>.</xsl:template>
  <xsl:template name="call"><xsl:param name="p"/><xsl:if test="$no and $p = 1">x</xsl:if>.</xsl:template>
  <xsl:template match="x | y" mode="learning"><xsl:param name="p" select="xs:decimal(if (self::y) then 'x' else '1')"/><xsl:if test="$no and ($p = 1 or $p = 2)">x</xsl:if>.</xsl:template>
</xsl:stylesheet>
