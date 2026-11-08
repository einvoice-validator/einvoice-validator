<?xml version="1.0" encoding="UTF-8"?>
<!-- Paths whose nodes would be sorted, where neither their order nor their duplicates
     matter: an effective boolean value (xsl:if, if, not, boolean, and, or, a predicate),
     exists, empty, a general comparison. Saxon-HE 12.10 reads them unsorted and lazily,
     the left operand in its own order: y before r, so that the node that decides is
     found before r/@b ('two', which cannot be compared with a number) is read. -->
<xsl:stylesheet version="2.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform"
                xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:f="urn:f">
  <xsl:output indent="no"/>
  <xsl:template match="/">
    <out>
      <if><xsl:if test="(r/y, r)/@*[. = 3]">yes</xsl:if></if>
      <exists><xsl:value-of select="exists((r/y, r)/@*[. = 3])"/></exists>
      <empty><xsl:value-of select="empty((r/y, r)/@*[. = 3])"/></empty>
      <not><xsl:value-of select="not((r/y, r)/@*[. = 3])"/></not>
      <boolean><xsl:value-of select="boolean((r/y, r)/@*[. = 3])"/></boolean>
      <or><xsl:value-of select="r/@a = 9 or (r/y, r)/@*[. = 3]"/></or>
      <and><xsl:value-of select="(r/y, r)/@*[. = 3] and true()"/></and>
      <cond><xsl:value-of select="if ((r/y, r)/@*[. = 3]) then 1 else 0"/></cond>
      <eq><xsl:value-of select="(r/y, r)/@* = 3"/></eq>
      <eq-right><xsl:value-of select="3 = (r/y, r)/@*"/></eq-right>
      <predicate><xsl:value-of select="count(r[(y, .)/@* = 3])"/></predicate>
      <nested><xsl:value-of select="exists((r/y, r)/@*[. = 3]/..)"/></nested>
    </out>
  </xsl:template>
</xsl:stylesheet>
