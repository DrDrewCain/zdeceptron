//! AWS Lambda: the hardest target, and therefore the one that proves the
//! design.
//!
//! Everything that makes the portability claim only half true is visible
//! here. Streaming needs `awslambda.streamifyResponse()`, a global the Node
//! runtime injects with no import and no standard counterpart, and it hands
//! the handler a **Node.js writable stream** rather than a WHATWG
//! `WritableStream`. Buffered mode cannot stream at all. Behind an ALB
//! nothing streams and the build is refused.
//!
//! And one thing that is commercial rather than technical: Lambda bills the
//! full duration of a streamed response and does not stop when the client
//! disconnects. `request.signal` never fires here. The generated idle
//! timeout is the only thing between a closed browser tab and a bill for
//! the rest of the function timeout.

use crate::capability::{Atomicity, Described, LiveSync};
use crate::cloudflare::slug;
use crate::{code_lines, File, LambdaFront, Options, Program};

const ENTRY: &str = include_str!("../js/lambda-entry.mjs");
const ENTRY_BUFFERED: &str = include_str!("../js/lambda-entry-buffered.mjs");
const STORE: &str = include_str!("../js/lambda-store.js");
const RUNTIME: &str = "nodejs22.x";

fn entry(options: &Options) -> &'static str {
    match options.front {
        LambdaFront::FunctionUrl
        | LambdaFront::ApiGatewayRestRegional
        | LambdaFront::ApiGatewayRestEdge => ENTRY,
        LambdaFront::Alb => ENTRY_BUFFERED,
    }
}

pub fn capabilities(options: &Options) -> Described {
    let front = match options.front {
        LambdaFront::FunctionUrl => {
            "a Lambda Function URL in `RESPONSE_STREAM` invoke mode. Response streaming is not \
             supported inside a VPC."
        }
        LambdaFront::ApiGatewayRestRegional => {
            "an API Gateway **REST** API (not HTTP, not WebSocket) with `STREAM` response \
             transfer mode, Regional endpoint."
        }
        LambdaFront::ApiGatewayRestEdge => {
            "an API Gateway **REST** API with `STREAM` response transfer mode, edge-optimized \
             endpoint — a 30-second idle timeout."
        }
        LambdaFront::Alb => {
            "an Application Load Balancer, buffered. Nothing streams here; a program with \
             durable state is refused at build time."
        }
    };

    let live_sync = match options.front {
        LambdaFront::FunctionUrl
        | LambdaFront::ApiGatewayRestRegional
        | LambdaFront::ApiGatewayRestEdge => LiveSync::Poll {
            reason: "DynamoDB has no push channel. Streams are pull-based change capture with a \
                     hard ceiling of two readers per shard, which cannot back one stream per \
                     browser tab",
        },
        LambdaFront::Alb => LiveSync::Impossible {
            reason: "an ALB takes one JSON response of at most 1 MB and does not honour \
                     `Transfer-Encoding`",
        },
    };

    let mut ceilings = vec![
        "Function timeout 900 s (15 minutes), hard. There is no longer budget for a stream."
            .to_string(),
        "**Lambda bills the full duration of a streamed response and is not interrupted when \
         the invoking client's connection is broken.** `request.signal` never fires. The \
         generated idle timeout is the mitigation and it is not optional."
            .to_string(),
        "The first 6 MB of a streamed response has uncapped bandwidth; after that Lambda \
         streams at a maximum of 2 MBps."
            .to_string(),
        "DynamoDB: item 400 KB, partition key 2048 bytes, sort key 1024 bytes, 3,000 RCU and \
         1,000 WCU per partition. Every durable signal is one partition key."
            .to_string(),
        "An atomic counter is not idempotent under retry. The adapter uses `SET n = \
         if_not_exists(n, :zero) + :delta`, which AWS recommends over `ADD` for exactly this \
         reason."
            .to_string(),
        "Node.js on Lambda has no SnapStart — it supports only Java, Python and .NET — so \
         initialisation cost is paid on every cold start unless you buy provisioned \
         concurrency."
            .to_string(),
    ];
    match options.front {
        LambdaFront::FunctionUrl => ceilings.push(
            "A Function URL documents no request-duration or idle limit of its own; the \
             function timeout is the only ceiling."
                .to_string(),
        ),
        LambdaFront::ApiGatewayRestRegional => ceilings.push(
            "API Gateway streams for at most 15 minutes, with a 5-minute idle timeout on a \
             Regional endpoint. Endpoint caching, content encoding and response transformation \
             are unavailable in `STREAM` mode."
                .to_string(),
        ),
        LambdaFront::ApiGatewayRestEdge => ceilings.push(
            "API Gateway streams for at most 15 minutes, with a **30-second** idle timeout on \
             an edge-optimized endpoint. The heartbeat is what keeps the stream alive."
                .to_string(),
        ),
        LambdaFront::Alb => ceilings.push(
            "ALB: response JSON at most 1 MB, no chunked transfer, upgrade requests rejected \
             with HTTP 400."
                .to_string(),
        ),
    }

    let mut manual = vec![
        "**The client bundle is not hosted.** This adapter serves `/_zd/*` only. Put `public/` \
         behind S3 and CloudFront, or in front of the same domain, and point the browser at \
         it."
        .to_string(),
        "Create the Secrets Manager secret the template references before the first deploy: \
         one JSON object holding every key listed below."
            .to_string(),
        "Package and deploy with `sam deploy --guided` from the deployment directory. Nothing \
         here has been deployed, and this tool cannot deploy it."
            .to_string(),
    ];
    match options.front {
        LambdaFront::FunctionUrl => {}
        LambdaFront::ApiGatewayRestRegional | LambdaFront::ApiGatewayRestEdge => manual.push(
            "Set the method's response transfer mode to `STREAM`. It is a method-level API \
             Gateway setting with no SAM or CloudFormation property, so it has to be set with \
             `aws apigateway update-integration` or in the console after the first deploy."
                .to_string(),
        ),
        LambdaFront::Alb => manual.push(
            "Create the target group and listener rule, register the function as a Lambda \
             target, and grant the load balancer permission to invoke it: `aws lambda \
             add-permission --principal elasticloadbalancing.amazonaws.com --action \
             lambda:InvokeFunction --source-arn <target-group-arn>`. None of that is in the \
             template, because none of it can be written before the target group exists."
                .to_string(),
        ),
    }

    (
        front.to_string(),
        live_sync,
        Atomicity::Native {
            mechanism: "DynamoDB `UpdateItem` with `SET n = if_not_exists(n, :zero) + :delta`, \
                        one round trip and no read",
        },
        "**wall clock, for the whole stream, including after the client leaves**",
        ceilings,
        manual,
        (code_lines(entry(options)), code_lines(STORE)),
    )
}

pub fn files(program: &Program<'_>, options: &Options) -> Vec<File> {
    vec![
        File::new("lambda.mjs", entry(options)),
        File::new("_zd/store.js", STORE),
        File::new(
            "package.json",
            "{\n  \"private\": true,\n  \"type\": \"module\"\n}\n",
        ),
        File::new("template.yaml", template(program, options)),
    ]
}

/// An AWS SAM template. SAM is used rather than raw CloudFormation because
/// `FunctionUrlConfig` with `InvokeMode: RESPONSE_STREAM` is one line here
/// and three resources there.
fn template(program: &Program<'_>, options: &Options) -> String {
    let app = slug(&options.app);
    let secret = format!("zd/{app}/secrets");

    let mut out = String::from(
        "# zdc · generated, do not edit\n\
         #\n\
         # No secret value appears in this file. Each environment key below is a\n\
         # CloudFormation dynamic reference, which resolves out of Secrets Manager at\n\
         # deploy time and is never persisted by CloudFormation.\n\
         AWSTemplateFormatVersion: '2010-09-09'\n\
         Transform: AWS::Serverless-2016-10-31\n",
    );
    out.push_str(&format!("Description: ZDeceptron deployment of {app}\n\n"));

    out.push_str("Resources:\n");
    out.push_str(
        "  ZdStore:\n\
         \x20   Type: AWS::DynamoDB::Table\n\
         \x20   Properties:\n\
         \x20     BillingMode: PAY_PER_REQUEST\n\
         \x20     AttributeDefinitions:\n\
         \x20       - AttributeName: k\n\
         \x20         AttributeType: S\n\
         \x20       - AttributeName: s\n\
         \x20         AttributeType: S\n\
         \x20     KeySchema:\n\
         \x20       - AttributeName: k\n\
         \x20         KeyType: HASH\n\
         \x20       - AttributeName: s\n\
         \x20         KeyType: RANGE\n\n",
    );

    if matches!(
        options.front,
        LambdaFront::ApiGatewayRestRegional | LambdaFront::ApiGatewayRestEdge
    ) {
        let endpoint = match options.front {
            LambdaFront::ApiGatewayRestRegional => "REGIONAL",
            LambdaFront::ApiGatewayRestEdge => "EDGE",
            LambdaFront::FunctionUrl | LambdaFront::Alb => "REGIONAL",
        };
        out.push_str(&format!(
            "  ZdApi:\n\
             \x20   Type: AWS::Serverless::Api\n\
             \x20   Properties:\n\
             \x20     StageName: live\n\
             \x20     EndpointConfiguration:\n\
             \x20       Type: {endpoint}\n\n"
        ));
    }

    out.push_str(
        "  ZdFunction:\n\
         \x20   Type: AWS::Serverless::Function\n\
         \x20   Properties:\n\
         \x20     CodeUri: ./\n\
         \x20     Handler: lambda.handler\n",
    );
    out.push_str(&format!("      Runtime: {RUNTIME}\n"));
    out.push_str("      MemorySize: 512\n");
    out.push_str(&format!(
        "      # The ceiling on a streamed response. Lambda bills all of it.\n      Timeout: {}\n",
        crate::capability::stream_budget(options)
            .ceiling_seconds()
            .max(1)
    ));
    out.push_str("      Environment:\n        Variables:\n");
    out.push_str("          ZD_TABLE: !Ref ZdStore\n");
    out.push_str("          ZD_REGION: !Ref AWS::Region\n");
    for key in program.environment {
        out.push_str(&format!(
            "          {key}: '{{{{resolve:secretsmanager:{secret}:SecretString:{key}}}}}'\n"
        ));
    }
    out.push_str(
        "      Policies:\n\
         \x20       - DynamoDBCrudPolicy:\n\
         \x20           TableName: !Ref ZdStore\n",
    );

    match options.front {
        LambdaFront::FunctionUrl => out.push_str(
            "      FunctionUrlConfig:\n\
             \x20       AuthType: NONE\n\
             \x20       # Buffered mode cannot stream: the response is delivered only when it\n\
             \x20       # is complete, capped at 6 MB. Live sync needs this line.\n\
             \x20       InvokeMode: RESPONSE_STREAM\n",
        ),
        LambdaFront::ApiGatewayRestRegional | LambdaFront::ApiGatewayRestEdge => out.push_str(
            "      Events:\n\
             \x20       Proxy:\n\
             \x20         Type: Api\n\
             \x20         Properties:\n\
             \x20           RestApiId: !Ref ZdApi\n\
             \x20           Path: /{proxy+}\n\
             \x20           Method: ANY\n",
        ),
        LambdaFront::Alb => {}
    }

    out.push_str("\nOutputs:\n");
    out.push_str("  Table:\n    Value: !Ref ZdStore\n");
    match options.front {
        LambdaFront::FunctionUrl => {
            out.push_str("  FunctionUrl:\n    Value: !GetAtt ZdFunctionUrl.FunctionUrl\n");
        }
        LambdaFront::ApiGatewayRestRegional | LambdaFront::ApiGatewayRestEdge => {
            out.push_str(
                "  ApiUrl:\n    Value: !Sub 'https://${ZdApi}.execute-api.${AWS::Region}.amazonaws.com/live'\n",
            );
        }
        LambdaFront::Alb => {
            out.push_str("  FunctionArn:\n    Value: !GetAtt ZdFunction.Arn\n");
        }
    }
    out
}
