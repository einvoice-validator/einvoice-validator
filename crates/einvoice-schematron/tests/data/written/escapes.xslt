<?xml version="1.0" encoding="UTF-8" standalone="no"?>
<xsl:stylesheet xmlns:svrl="http://purl.oclc.org/dsdl/svrl" xmlns:xsl="http://www.w3.org/1999/XSL/Transform" version="2.0">
  <xsl:template match="a[@b = &quot;c&quot; or @d = '&lt;&amp;>']" name="tab&#x9;lf&#xA;cr&#xD;end">
    <xsl:text>&lt; &amp; &gt; ]]&gt; " ' tab	lf
cr&#xD;crlf&#xD;
del&#x7F;nel&#x85;c1&#x9F;nbsp é€😀𝄞</xsl:text>
    <!-- comment with < & > ' " é 😀 -->
<?pi data with < & > ' "?>
  </xsl:template>
</xsl:stylesheet>
