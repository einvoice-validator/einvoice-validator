<?xml version="1.0" encoding="UTF-8" standalone="no"?>
<stylesheet xmlns:svrl="http://purl.oclc.org/dsdl/svrl" xmlns="http://www.w3.org/1999/XSL/Transform" version="2.0">
  <template match="/">
    <value-of select="1" />
    <plain xmlns="" a="1">
      <inner />
      <ns0:in xmlns:ns0="urn:x" />
    </plain>
    <again>
      <plain xmlns="">
        <ns0:deeper xmlns:ns0="urn:d">
          <plain />
        </ns0:deeper>
      </plain>
    </again>
  </template>
</stylesheet>
