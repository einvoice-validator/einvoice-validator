<?xml version="1.0" encoding="UTF-8"?>
<!-- `cast as`: target types, the optional `?`, atomization of nodes, whitespace, the
     up-cast of an xs:integer to xs:decimal, precedence, and casts in predicates (numeric
     ones are positional). -->
<xsl:stylesheet version="2.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform" xmlns:xs="http://www.w3.org/2001/XMLSchema">
  <xsl:output indent="no"/>
  <xsl:template match="/">
    <out>
      <v n="1"><xsl:value-of select="'1' cast as xs:boolean"/></v>
      <v n="2"><xsl:value-of select="' true ' cast as xs:boolean"/></v>
      <v n="3"><xsl:value-of select="'0' cast as xs:boolean = false()"/></v>
      <v n="4"><xsl:value-of select="/r/@a cast as xs:integer + 1"/></v>
      <v n="5"><xsl:value-of select="/r/amounts/amount[1] cast as xs:decimal"/></v>
      <v n="6"><xsl:value-of select="/r/amounts/amount[3] cast as xs:decimal * 2"/></v>
      <v n="7"><xsl:value-of select="(/r/amounts/amount[2] cast as xs:integer) cast as xs:decimal div 3"/></v>
      <v n="8"><xsl:value-of select="(2 cast as xs:decimal) div 3"/></v>
      <v n="9"><xsl:value-of select="(/r/amounts/amount[2] cast as xs:decimal) div 3"/></v>
      <v n="10"><xsl:value-of select="count(/r/missing cast as xs:boolean?)"/></v>
      <v n="11"><xsl:value-of select="10.50 cast as xs:string"/></v>
      <v n="12"><xsl:value-of select="2.7 cast as xs:integer, -2.7 cast as xs:integer"/></v>
      <v n="13"><xsl:value-of select="'2013-04-10' cast as xs:date"/></v>
      <v n="14"><xsl:value-of select="('2013-04-10' cast as xs:date) cast as xs:string"/></v>
      <v n="15"><xsl:value-of select="1 cast as xs:boolean, 0.0 cast as xs:boolean, 'NaN' cast as xs:string"/></v>
      <v n="17"><xsl:value-of select="('true' cast as xs:boolean) instance of xs:boolean, (1 cast as xs:decimal) instance of xs:integer"/></v>
      <v n="18"><xsl:value-of select="-/r/@a cast as xs:integer"/></v>
      <v n="19"><xsl:value-of select="/r/amounts/amount[. cast as xs:decimal gt 2]"/></v>
      <v n="20"><xsl:value-of select="/r/x[@n cast as xs:integer]/@n"/></v>
      <v n="21"><xsl:value-of select="/r/x[(@n cast as xs:integer?) = 2]/@n"/></v>
      <v n="22"><xsl:value-of select="/r/x[@n cast as xs:string]/@n"/></v>
      <v n="23"><xsl:value-of select="/r/@a cast as xs:integer castable as xs:boolean"/></v>
      <v n="24"><xsl:value-of select="(/r/amounts/amount[2] cast as xs:integer) * 1.5"/></v>
      <v n="25"><xsl:value-of select="sum(/r/amounts/amount/(. cast as xs:decimal))"/></v>
      <v n="26"><xsl:value-of select="(/r/missing cast as xs:boolean = true()) and false()"/></v>
      <v n="27"><xsl:value-of select="false() and (/r/missing cast as xs:boolean)"/></v>
      <v n="28"><xsl:value-of select="'-0' cast as xs:decimal, '+.5' cast as xs:decimal, '5.' cast as xs:decimal"/></v>
      <v n="29"><xsl:value-of select="'007' cast as xs:integer, ' -3 ' cast as xs:integer"/></v>
      <v n="30"><xsl:value-of select="if (/r/x[1] cast as xs:string = 't1') then 'yes' else 'no'"/></v>
    </out>
  </xsl:template>
</xsl:stylesheet>
