<?xml version="1.0" encoding="UTF-8"?>
<!-- Error: $year is referred to three times, all in the value of $isLeapYear, which Saxon
     inlines into $maxDay, which it inlines into the result. Saxon does not count the
     references of $year again ($month, right after it, stays), finds none left, and
     computes its value of one item where it is bound, though nothing reads it here. -->
<xsl:stylesheet version="2.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform" xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:f="urn:f">
  <xsl:function name="f:date" as="xs:boolean">
    <xsl:param name="date" as="xs:string?"/>
    <xsl:variable name="year" select="xs:decimal(substring($date, 1, 4))"/>
    <xsl:variable name="month" select="xs:decimal(substring($date, 6, 2))"/>
    <xsl:variable name="isLeapYear" select="($year mod 4 = 0 and $year mod 100 != 0) or ($year mod 400 = 0)"/>
    <xsl:variable name="maxDay" select="if ($month = (1, 3, 5)) then 31 else if ($month = 2 and $isLeapYear) then 29 else 0"/>
    <xsl:sequence select="matches($date, '^20') and 1 le $maxDay"/>
  </xsl:function>
  <xsl:template match="/"><out><xsl:value-of select="f:date(concat('08/02/2018', /r/@b))"/></out></xsl:template>
</xsl:stylesheet>
