<?xml version="1.0" encoding="UTF-8" standalone="no"?>
<xsl:stylesheet xmlns:ns0="urn:zero" xmlns:svrl="urn:not-svrl" xmlns:u="urn:u" xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:xsd="http://www.w3.org/2001/XMLSchema" xmlns:xsl="http://www.w3.org/1999/XSL/Transform" xmlns:z="urn:zero" version="2.0">
  <xsl:template match="/">
    <xsd:element xsd:type="t" />
    <ns0:in xmlns:ns0="urn:a" ns0:att="1" xmlns:ns1="urn:b" ns1:att="2" plain="3">
      <ns2:deep xmlns:ns2="urn:c">
        <ns0:again />
        <svrl:x />
        <ns0:other xml:lang="fr" xml:space="preserve">
          <u:y />
        </ns0:other>
      </ns2:deep>
    </ns0:in>
    <z:z z:k="v" />
    <ns0:d xmlns:ns0="urn:d" ns0:attr="in the default namespace" />
  </xsl:template>
</xsl:stylesheet>
